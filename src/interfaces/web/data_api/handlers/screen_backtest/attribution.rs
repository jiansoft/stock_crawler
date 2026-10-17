//! `/stocks/screen/backtest/attribution` handler：把策略相對類 0050 基準的超額報酬拆成產業配置、選股
//! 與交互三項（Brinson-Fachler），看落後是因為少配了某個產業，還是同產業選到的股票比較差。
//!
//! 基準：每個換股日同市場母體（相對強度母體，上市櫃普通股）中近 20 個交易日平均成交金額最高的
//! [`BENCHMARK_SIZE`] 檔，依平均成交金額加權（近似 0050 的市值加權；歷史成分股與權重沒有留存）。
//! 只看策略部位（不含 `etf_weight` 的 0050 核心部位），報酬未扣交易成本；策略部位的現金（大盤濾網
//! 空手、單檔上限分不完）是一個報酬 0 的「現金」類別，基準權重 0，效果全算在配置。
//!
//! 每期各類別 i（策略權重 wp、報酬 rp；基準權重 wb、報酬 rb；基準整體報酬 Rb）：
//! 配置 = (wp − wb)(rb − Rb)、選股 = wb(rp − rb)、交互 = (wp − wb)(rp − rb)，三項合計等於 Rp − Rb。
//! 策略沒持有的產業 rp 取 rb（選股、交互為 0）；基準沒有的產業 rb 取 Rb（配置為 0）。整段與各年度
//! 是各期效果直接相加，不複利。

use std::collections::{BTreeMap, HashMap};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, NaiveDate};

use super::preset::BacktestQuery;
use super::{Backtest, PeriodRow, portfolio};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    BacktestAttributionTotals, BacktestAttributionYear, BacktestIndustryEffect, ErrorBody,
    ScreenBacktestAttributionResponse, ScreenBacktestParams,
};
use crate::interfaces::web::data_api::handlers::{database_error, error_response, round2};

/// 基準的檔數。
pub(super) const BENCHMARK_SIZE: i64 = 50;

/// 基準先取換股日當天成交金額前幾名，再依 20 日平均成交金額取前 [`BENCHMARK_SIZE`] 檔。
pub(super) const PRESELECT_SIZE: i64 = 300;

/// 每年列出影響最大的幾個產業。
const YEAR_INDUSTRIES: usize = 5;

/// 每個換股日（`$3`）同市場（`$5`，0 為上市櫃全部）母體中近 20 個交易日平均成交金額最高的 `$6` 檔
/// 的產業、`$4` 日報酬與平均成交金額，依成交金額由高到低；沒有報酬或成交金額的股票不列入。`$1`／`$2`
/// 為換股日的最早與最晚一天，往前多讀 60 個日曆天讓第一期也有 20 個交易日。
///
/// 逐檔算 20 日平均很慢（持有 5 日、近八年要 20 秒以上），所以先取換股日當天成交金額前 `$7`
/// （[`PRESELECT_SIZE`]）名、只對它們算平均；20 日平均排進前 50 的股票當天不在前 300 名的情況
/// 極少見。
pub(super) const BENCHMARK_SQL: &str = r#"
WITH days AS (
    SELECT date FROM daily_relative_strength
    WHERE date BETWEEN $1::date - 60 AND $2
    GROUP BY date
),
windowed AS (
    SELECT date,
           COALESCE(LAG(date, 19) OVER (ORDER BY date), MIN(date) OVER (ORDER BY date))
               AS liquidity_start
    FROM days
),
active AS (
    SELECT w.date, w.liquidity_start, rs.stock_symbol, COALESCE(s.stock_industry_id, 0) AS industry,
           (CASE $4::int
                WHEN 5 THEN f.return_5d
                WHEN 20 THEN f.return_20d
                WHEN 60 THEN f.return_60d
                WHEN 120 THEN f.return_120d
                ELSE f.return_240d
            END)::float8 AS ret,
           ROW_NUMBER() OVER (PARTITION BY w.date ORDER BY q."TradeValue" DESC, rs.stock_symbol)
               AS day_rank
    FROM windowed w
    JOIN daily_relative_strength rs ON rs.date = w.date
    JOIN stocks s ON s.stock_symbol = rs.stock_symbol
    JOIN daily_forward_return f ON f.date = w.date AND f.stock_symbol = rs.stock_symbol
    JOIN "DailyQuotes" q ON q."Date" = w.date AND q.stock_symbol = rs.stock_symbol
    WHERE w.date = ANY($3)
      AND ($5 = 0 OR s.stock_exchange_market_id = $5)
),
universe AS (
    SELECT a.date, a.stock_symbol, a.industry, a.ret,
           (SELECT AVG(q."TradeValue")::float8 FROM "DailyQuotes" q
            WHERE q.stock_symbol = a.stock_symbol
              AND q."Date" BETWEEN a.liquidity_start AND a.date) AS trade_value
    FROM active a
    WHERE a.ret IS NOT NULL AND a.day_rank <= $7
),
ranked AS (
    SELECT date, industry, ret, trade_value,
           ROW_NUMBER() OVER (PARTITION BY date ORDER BY trade_value DESC, stock_symbol) AS r
    FROM universe
    WHERE trade_value > 0
)
SELECT date,
       array_agg(industry::int4 ORDER BY r) AS industries,
       array_agg(ret ORDER BY r) AS returns,
       array_agg(trade_value ORDER BY r) AS trade_values
FROM ranked
WHERE r <= $6
GROUP BY date
ORDER BY date
"#;

/// 產業名稱。
const INDUSTRY_NAMES_SQL: &str =
    "SELECT stock_industry_id, name::text FROM stock_industry WHERE stock_industry_id = ANY($1)";

/// [`BENCHMARK_SQL`] 的一列：一個換股日的基準成分股。
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
pub(super) struct BenchmarkRow {
    pub(super) date: NaiveDate,
    pub(super) industries: Vec<i32>,
    pub(super) returns: Vec<f64>,
    pub(super) trade_values: Vec<f64>,
}

/// 歸因的類別：產業或策略部位的現金。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Bucket {
    Industry(i32),
    Cash,
}

/// 一期一個類別的權重（0–1）與效果（百分點）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Effect {
    pub(super) weight: f64,
    pub(super) benchmark_weight: f64,
    pub(super) allocation: f64,
    pub(super) selection: f64,
    pub(super) interaction: f64,
}

impl Effect {
    fn total(&self) -> f64 {
        self.allocation + self.selection + self.interaction
    }

    fn add(&mut self, other: &Self) {
        self.weight += other.weight;
        self.benchmark_weight += other.benchmark_weight;
        self.allocation += other.allocation;
        self.selection += other.selection;
        self.interaction += other.interaction;
    }
}

/// 一期的歸因：策略與基準的報酬（%）、0050 報酬與各類別的效果。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PeriodAttribution {
    pub(super) date: NaiveDate,
    pub(super) strategy_return: f64,
    pub(super) benchmark_return: f64,
    pub(super) etf_return: Option<f64>,
    pub(super) effects: BTreeMap<Bucket, Effect>,
}

/// 各類別的權重合計與加權報酬合計。
fn group(items: impl Iterator<Item = (Bucket, f64, f64)>) -> BTreeMap<Bucket, (f64, f64)> {
    let mut groups: BTreeMap<Bucket, (f64, f64)> = BTreeMap::new();
    for (bucket, weight, ret) in items {
        let entry = groups.entry(bucket).or_default();
        entry.0 += weight;
        entry.1 += weight * ret;
    }
    groups
}

/// 一期的 Brinson-Fachler 歸因。策略權重為持股比例（見 [`portfolio::shares`]），只算有報酬的持股
/// 並把它們的權重放大到持股比例合計（與平均報酬的算法相同），其餘為現金；基準沒有成分股回 `None`。
pub(super) fn attribute(row: &PeriodRow, benchmark: &BenchmarkRow) -> Option<PeriodAttribution> {
    let total_value: f64 = benchmark.trade_values.iter().sum();
    if total_value <= 0.0 {
        return None;
    }
    let bench = group(
        benchmark
            .industries
            .iter()
            .zip(&benchmark.returns)
            .zip(&benchmark.trade_values)
            .map(|((industry, ret), value)| {
                (Bucket::Industry(*industry), value / total_value, *ret)
            }),
    );
    let benchmark_return: f64 = bench.values().map(|(_, weighted)| weighted).sum();

    let shares = portfolio::shares(row);
    let priced: Vec<(Bucket, f64, f64)> = row
        .returns
        .iter()
        .enumerate()
        .filter_map(|(index, ret)| {
            let industry = row.industries.get(index).copied().unwrap_or(0);
            ret.map(|ret| (Bucket::Industry(industry), shares[index], ret))
        })
        .collect();
    let priced_share: f64 = priced.iter().map(|(_, share, _)| share).sum();
    let invested = if priced_share > 0.0 {
        1.0 - row.cash
    } else {
        0.0
    };
    let mut strategy = group(
        priced
            .into_iter()
            .map(|(bucket, share, ret)| (bucket, share / priced_share * invested, ret)),
    );
    if invested < 1.0 {
        strategy.insert(Bucket::Cash, (1.0 - invested, 0.0));
    }
    let strategy_return: f64 = strategy.values().map(|(_, weighted)| weighted).sum();

    let mut effects = BTreeMap::new();
    for bucket in strategy.keys().chain(bench.keys()) {
        if effects.contains_key(bucket) {
            continue;
        }
        let (wp, weighted_p) = strategy.get(bucket).copied().unwrap_or_default();
        let (wb, weighted_b) = bench.get(bucket).copied().unwrap_or_default();
        let rb = match bucket {
            Bucket::Cash => 0.0,
            Bucket::Industry(_) if wb > 0.0 => weighted_b / wb,
            Bucket::Industry(_) => benchmark_return,
        };
        let rp = if wp > 0.0 { weighted_p / wp } else { rb };
        effects.insert(
            *bucket,
            Effect {
                weight: wp,
                benchmark_weight: wb,
                allocation: (wp - wb) * (rb - benchmark_return),
                selection: wb * (rp - rb),
                interaction: (wp - wb) * (rp - rb),
            },
        );
    }
    Some(PeriodAttribution {
        date: row.date,
        strategy_return,
        benchmark_return,
        etf_return: row.etf_return,
        effects,
    })
}

/// 各期合計：期數、報酬與三項效果相加，以及各類別的效果與權重合計。
pub(super) fn totals(
    periods: &[&PeriodAttribution],
) -> (BacktestAttributionTotals, BTreeMap<Bucket, Effect>) {
    let mut buckets: BTreeMap<Bucket, Effect> = BTreeMap::new();
    for period in periods {
        for (bucket, effect) in &period.effects {
            buckets.entry(*bucket).or_default().add(effect);
        }
    }
    let sum = |pick: fn(&Effect) -> f64| round2(buckets.values().map(pick).sum());
    let strategy_return: f64 = periods.iter().map(|period| period.strategy_return).sum();
    let benchmark_return: f64 = periods.iter().map(|period| period.benchmark_return).sum();
    let has_etf = periods.iter().any(|period| period.etf_return.is_some());
    let totals = BacktestAttributionTotals {
        periods: periods.len() as i64,
        strategy_return: round2(strategy_return),
        benchmark_return: round2(benchmark_return),
        etf_return: has_etf
            .then(|| round2(periods.iter().filter_map(|period| period.etf_return).sum())),
        excess_return: round2(strategy_return - benchmark_return),
        allocation: sum(|effect| effect.allocation),
        selection: sum(|effect| effect.selection),
        interaction: sum(|effect| effect.interaction),
    };
    (totals, buckets)
}

/// 各類別的效果輸出：權重換成各期平均（%），依三項合計由大到小；`top` 有值時改依合計的絕對值
/// 由大到小、只取前 `top` 個。
pub(super) fn industry_effects(
    buckets: &BTreeMap<Bucket, Effect>,
    periods: usize,
    names: &HashMap<i32, String>,
    top: Option<usize>,
) -> Vec<BacktestIndustryEffect> {
    let count = periods.max(1) as f64;
    let mut items: Vec<(f64, BacktestIndustryEffect)> = buckets
        .iter()
        .map(|(bucket, effect)| {
            let industry = match bucket {
                Bucket::Cash => "現金".to_owned(),
                Bucket::Industry(id) => names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "未分類".to_owned()),
            };
            (
                effect.total(),
                BacktestIndustryEffect {
                    industry,
                    avg_weight: round2(effect.weight / count * 100.0),
                    benchmark_avg_weight: round2(effect.benchmark_weight / count * 100.0),
                    allocation: round2(effect.allocation),
                    selection: round2(effect.selection),
                    interaction: round2(effect.interaction),
                    total: round2(effect.total()),
                },
            )
        })
        .collect();
    match top {
        Some(top) => {
            items.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
            items.truncate(top);
        }
        None => items.sort_by(|a, b| b.0.total_cmp(&a.0)),
    }
    items.into_iter().map(|(_, item)| item).collect()
}

/// 依換股日年度的合計與影響最大的 [`YEAR_INDUSTRIES`] 個產業，由舊到新。
pub(super) fn attribution_years(
    periods: &[PeriodAttribution],
    names: &HashMap<i32, String>,
) -> Vec<BacktestAttributionYear> {
    let mut by_year: BTreeMap<i32, Vec<&PeriodAttribution>> = BTreeMap::new();
    for period in periods {
        by_year.entry(period.date.year()).or_default().push(period);
    }
    by_year
        .into_iter()
        .map(|(year, periods)| {
            let (totals, buckets) = totals(&periods);
            BacktestAttributionYear {
                year,
                industries: industry_effects(&buckets, periods.len(), names, Some(YEAR_INDUSTRIES)),
                totals,
            }
        })
        .collect()
}

/// 策略相對類 0050 基準的報酬歸因：配置、選股與交互效果，整段、依年度與依產業。
///
/// 參數與 `/stocks/screen/backtest` 相同（含 `rebalance_offset`、停損）；`etf_weight` 與 `split` 不使用，
/// 只看策略部位。
///
/// # Errors
///
/// 參數驗證與 `/stocks/screen/backtest` 相同（422）；迄日之前沒有相對強度資料回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest/attribution", tag = "data-api", params(ScreenBacktestParams), responses((status = 200, body = ScreenBacktestAttributionResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest_attribution(BacktestQuery(params): BacktestQuery) -> Response {
    let backtest = match Backtest::prepare(&params).await {
        Ok(backtest) => backtest,
        Err(response) => return *response,
    };
    let rebalance_offset = params.rebalance_offset.unwrap_or(0);
    if rebalance_offset >= backtest.hold_days {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rebalance_offset 必須介於 0 至 hold_days − 1",
        );
    }
    let raw = match backtest.run(rebalance_offset).await {
        Ok(raw) => raw,
        Err(error) => return database_error(error),
    };
    let rows: Vec<&PeriodRow> = raw.rows.iter().filter(|row| !row.pending).collect();
    let benchmarks = match benchmark_rows(&backtest, &rows).await {
        Ok(benchmarks) => benchmarks,
        Err(error) => return database_error(error),
    };
    let periods: Vec<PeriodAttribution> = rows
        .iter()
        .filter_map(|row| attribute(row, benchmarks.get(&row.date)?))
        .collect();
    let ids: Vec<i32> = periods
        .iter()
        .flat_map(|period| period.effects.keys())
        .filter_map(|bucket| match bucket {
            Bucket::Industry(id) => Some(*id),
            Bucket::Cash => None,
        })
        .collect();
    let names: HashMap<i32, String> = match sqlx::query_as(INDUSTRY_NAMES_SQL)
        .bind(&ids)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(names) => names.into_iter().collect(),
        Err(error) => return database_error(error),
    };
    let all: Vec<&PeriodAttribution> = periods.iter().collect();
    let (summary, buckets) = totals(&all);

    Json(ScreenBacktestAttributionResponse {
        from: backtest.from.to_string(),
        to: backtest.to.to_string(),
        market: backtest.market.to_owned(),
        hold_days: backtest.hold_days,
        limit: backtest.limit,
        rank_by: backtest.rank_by.to_owned(),
        weighting: backtest.weighting.as_str().to_owned(),
        max_weight: backtest.max_weight,
        stop_loss: params.stop_loss,
        trailing_stop: params.trailing_stop,
        rebalance_offset,
        benchmark_size: BENCHMARK_SIZE,
        industries: industry_effects(&buckets, periods.len(), &names, None),
        years: attribution_years(&periods, &names),
        summary,
    })
    .into_response()
}

/// 各換股日的基準成分股；沒有期數時不查詢。
async fn benchmark_rows(
    backtest: &Backtest<'_>,
    rows: &[&PeriodRow],
) -> Result<HashMap<NaiveDate, BenchmarkRow>, sqlx::Error> {
    let dates: Vec<NaiveDate> = rows.iter().map(|row| row.date).collect();
    let (Some(first), Some(last)) = (dates.first(), dates.last()) else {
        return Ok(HashMap::new());
    };
    let benchmarks: Vec<BenchmarkRow> = sqlx::query_as(BENCHMARK_SQL)
        .bind(first)
        .bind(last)
        .bind(&dates)
        .bind(i32::from(backtest.hold_days))
        .bind(backtest.market_id)
        .bind(BENCHMARK_SIZE)
        .bind(PRESELECT_SIZE)
        .fetch_all(database::get_batch_connection())
        .await?;
    Ok(benchmarks.into_iter().map(|row| (row.date, row)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, m, d).expect("有效日期")
    }

    fn near(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-9
    }

    /// 策略：產業 1 的 A（報酬 10）、產業 2 的 B（報酬 −2），等權。
    fn strategy() -> PeriodRow {
        PeriodRow {
            date: day(1, 2),
            holdings: 2,
            symbols: vec!["A".to_owned(), "B".to_owned()],
            returns: vec![Some(10.0), Some(-2.0)],
            industries: vec![1, 2],
            avg_return: Some(4.0),
            etf_return: Some(3.0),
            ..PeriodRow::default()
        }
    }

    /// 基準：產業 1 占 75%（報酬 4）、產業 3 占 25%（報酬 8）；整體 5。
    fn benchmark() -> BenchmarkRow {
        BenchmarkRow {
            date: day(1, 2),
            industries: vec![1, 3, 1],
            returns: vec![4.0, 8.0, 4.0],
            trade_values: vec![50.0, 25.0, 25.0],
        }
    }

    /// 三項效果合計等於超額報酬；策略沒持有的產業 rp 取 rb、基準沒有的產業 rb 取 Rb。
    #[test]
    fn attribute_splits_excess_return() {
        let result = attribute(&strategy(), &benchmark()).expect("有基準");
        assert!(near(result.strategy_return, 4.0));
        assert!(near(result.benchmark_return, 5.0));
        assert_eq!(result.etf_return, Some(3.0));
        let sum: f64 = result.effects.values().map(Effect::total).sum();
        assert!(near(sum, -1.0), "{result:?}");

        let one = result.effects[&Bucket::Industry(1)];
        assert!(near(one.weight, 0.5) && near(one.benchmark_weight, 0.75));
        assert!(near(one.allocation, (0.5 - 0.75) * (4.0 - 5.0)));
        assert!(near(one.selection, 0.75 * (10.0 - 4.0)));
        assert!(near(one.interaction, (0.5 - 0.75) * (10.0 - 4.0)));
        // 基準沒有產業 2：rb 取 Rb，配置為 0、差額都在交互。
        let two = result.effects[&Bucket::Industry(2)];
        assert!(near(two.allocation, 0.0) && near(two.selection, 0.0));
        assert!(near(two.interaction, 0.5 * (-2.0 - 5.0)));
        // 策略沒持有產業 3：選股、交互為 0，少配的配置效果。
        let three = result.effects[&Bucket::Industry(3)];
        assert!(near(three.allocation, -0.25 * (8.0 - 5.0)));
        assert!(near(three.selection, 0.0) && near(three.interaction, 0.0));
        assert!(!result.effects.contains_key(&Bucket::Cash));

        assert!(attribute(&strategy(), &BenchmarkRow::default()).is_none());
    }

    /// 現金（單檔上限、空手、沒有報酬的持股）是報酬 0 的類別，效果全在配置；仍滿足合計等於超額報酬。
    #[test]
    fn attribute_treats_cash_as_zero_return_bucket() {
        let capped = PeriodRow {
            cash: 0.2,
            ..strategy()
        };
        let result = attribute(&capped, &benchmark()).expect("有基準");
        assert!(near(result.strategy_return, 3.2));
        let cash = result.effects[&Bucket::Cash];
        assert!(near(cash.weight, 0.2));
        assert!(near(cash.allocation, 0.2 * -5.0));
        let sum: f64 = result.effects.values().map(Effect::total).sum();
        assert!(near(sum, 3.2 - 5.0));

        // 一檔沒有報酬：另一檔放大到全部持股比例。
        let unpriced = PeriodRow {
            returns: vec![Some(10.0), None],
            ..strategy()
        };
        let result = attribute(&unpriced, &benchmark()).expect("有基準");
        assert!(near(result.strategy_return, 10.0));
        assert!(near(result.effects[&Bucket::Industry(1)].weight, 1.0));

        // 空手：全部是現金。
        let idle = PeriodRow {
            holdings: 0,
            symbols: Vec::new(),
            returns: Vec::new(),
            industries: Vec::new(),
            avg_return: None,
            ..strategy()
        };
        let result = attribute(&idle, &benchmark()).expect("有基準");
        assert!(near(result.strategy_return, 0.0));
        assert!(near(result.effects[&Bucket::Cash].allocation, -5.0));
    }

    /// 合計與產業輸出：各期相加、權重為各期平均；整段依合計排序、年度依絕對值取前幾個；名稱查無為
    /// 「未分類」、現金為「現金」；0050 全缺為 null。
    #[test]
    fn totals_and_industry_effects_aggregate_periods() {
        let first = attribute(&strategy(), &benchmark()).expect("有基準");
        let second = PeriodAttribution {
            date: day(2, 1),
            etf_return: None,
            ..first.clone()
        };
        let (summary, buckets) = totals(&[&first, &second]);
        assert_eq!(summary.periods, 2);
        assert_eq!(summary.strategy_return, 8.0);
        assert_eq!(summary.benchmark_return, 10.0);
        assert_eq!(summary.etf_return, Some(3.0));
        assert_eq!(summary.excess_return, -2.0);
        assert!(near(
            summary.allocation + summary.selection + summary.interaction,
            -2.0
        ));
        let (empty, _) = totals(&[]);
        assert_eq!((empty.periods, empty.etf_return), (0, None));

        let names: HashMap<i32, String> = [(1, "半導體業".to_owned())].into_iter().collect();
        let all = industry_effects(&buckets, 2, &names, None);
        assert_eq!(all[0].industry, "半導體業");
        assert_eq!(all[0].avg_weight, 50.0);
        assert_eq!(all[0].benchmark_avg_weight, 75.0);
        assert!(all.windows(2).all(|pair| pair[0].total >= pair[1].total));
        assert!(all.iter().any(|item| item.industry == "未分類"));
        // 依絕對值：產業 2（合計 −7）大於產業 1（6.5）。
        let top = industry_effects(&buckets, 2, &names, Some(1));
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].industry, "未分類");
        assert_eq!(top[0].total, -7.0);

        let mut cash = BTreeMap::new();
        cash.insert(Bucket::Cash, Effect::default());
        assert_eq!(industry_effects(&cash, 0, &names, None)[0].industry, "現金");

        let years = attribution_years(&[first, second], &names);
        assert_eq!(years.len(), 1);
        assert_eq!(years[0].year, 2024);
        assert_eq!(years[0].totals.periods, 2);
        assert!(years[0].industries.len() <= YEAR_INDUSTRIES);
    }
}
