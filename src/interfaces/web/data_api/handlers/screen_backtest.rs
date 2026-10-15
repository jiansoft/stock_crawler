//! `/stocks/screen/backtest` handler：依每日留存的條件定期選股，回測等權持有的報酬。
//!
//! 每隔 `hold_days` 個交易日換股一次：在換股日從相對強度母體（上市櫃普通股，見
//! `daily_relative_strength`）挑出符合條件的股票，依相對強度分數由高到低取前 `limit` 檔，
//! 收盤等權買進、持有 `hold_days` 個交易日；該期報酬取 `daily_forward_return` 的同天期還原報酬，
//! 期間內下市的股票算到最後一筆收盤。只用當天就已知道的資料（相對強度、法人連續買超、技術訊號），
//! 沒有前視偏誤；相對強度母體看當時有沒有在交易，沒有存活者偏差。
//!
//! 交易成本依換股時實際賣出、買進的比例扣除：買賣各付手續費 0.1425%（乘上 `fee_discount`），
//! 賣出另付證交稅 0.3%；同一檔續抱不計成本，也不計等權再平衡的小額調整。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

use super::signal_history::{chip_streak_ctes, validate_rs_range, validate_streaks};
use super::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    round2, round4, signal_filter_error,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    BacktestPeriod, BacktestSummary, ErrorBody, ScreenBacktestParams, ScreenBacktestResponse,
};

/// 可選的持有天數，與 `daily_forward_return` 的報酬欄位對應。
const HOLD_DAYS: [u16; 5] = [5, 20, 60, 120, 240];

/// 預設回測期間：迄日往前 1,095 天（約三年）。
const DEFAULT_PERIOD_DAYS: i64 = 1095;

/// 回測期間上限（日曆天，含起訖）。
const MAX_PERIOD_DAYS: i64 = 3653;

/// 每年約有幾個交易日，用來把每期報酬換算成年化報酬。
const TRADING_DAYS_PER_YEAR: f64 = 245.0;

/// 手續費率（%），買賣各付一次；未打折。
const FEE_RATE: f64 = 0.1425;

/// 股票證交稅率（%），賣出時付。
const TAX_RATE: f64 = 0.3;

/// 輸出每期前幾檔代號。
const LISTED_SYMBOLS: usize = 10;

/// `$1`（含）之前最近一個有相對強度的交易日；`$1` 為 `NULL` 時取最新。
const LATEST_RS_DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_relative_strength WHERE $1::date IS NULL OR date <= $1
"#;

/// 回測 SQL 的主體。`$1`／`$2` 起訖、`$3` 市場 id（0 為上市櫃全部）、`$4` 持有天數、`$5` 每期檔數、
/// `$6` 產業、`$7`／`$8` 相對強度上下限、`$9`／`$10` 訊號來源與種類；籌碼版另有 `$11`／`$12`。
///
/// - `days`：有相對強度的交易日（往前多讀 400 個日曆天，讓第一期也能回看訊號）。
/// - `rebal`：起日之後每隔 `$4` 個交易日的換股日；`window_start` 是含當天往前第 `$4` 個交易日，
///   訊號條件看 `window_start`～換股日。
/// - `bench`：同一天同市場全部母體股票的平均報酬；`samples` 為 0 代表還沒滿持有天數，不列入。
macro_rules! backtest_sql {
    ($with:literal, $chip_join:literal, $chip_filter:literal) => {
        concat!(
            $with,
            r#"
days AS (
    SELECT date FROM daily_relative_strength
    WHERE date BETWEEN $1::date - 400 AND $2
    GROUP BY date
),
windowed AS (
    SELECT date, LAG(date, $4::int - 1, date) OVER (ORDER BY date) AS window_start
    FROM days
),
rebal AS (
    SELECT date, window_start
    FROM (
        SELECT date, window_start, ROW_NUMBER() OVER (ORDER BY date) - 1 AS i
        FROM windowed
        WHERE date >= $1
    ) AS numbered
    WHERE i % $4::int = 0
),
universe AS (
    SELECT rb.date, rb.window_start, rs.stock_symbol, rs.rs_score, s.stock_industry_id,
           CASE $4::int
               WHEN 5 THEN f.return_5d
               WHEN 20 THEN f.return_20d
               WHEN 60 THEN f.return_60d
               WHEN 120 THEN f.return_120d
               ELSE f.return_240d
           END AS ret
    FROM rebal rb
    JOIN daily_relative_strength rs ON rs.date = rb.date
    JOIN stocks s ON s.stock_symbol = rs.stock_symbol
    LEFT JOIN daily_forward_return f ON f.date = rb.date AND f.stock_symbol = rs.stock_symbol
    WHERE ($3 = 0 OR s.stock_exchange_market_id = $3)
),
bench AS (
    SELECT date, AVG(ret)::float8 AS avg_ret, COUNT(ret) AS samples
    FROM universe
    GROUP BY date
),
picks AS (
    SELECT u.date, u.stock_symbol, u.ret,
           ROW_NUMBER() OVER (PARTITION BY u.date ORDER BY u.rs_score DESC, u.stock_symbol) AS pick_rank
    FROM universe u
    LEFT JOIN stock_industry si ON si.stock_industry_id = u.stock_industry_id"#,
            $chip_join,
            r#"
    WHERE ($6::text IS NULL OR si.name = $6)
      AND ($7::smallint IS NULL OR u.rs_score >= $7)
      AND ($8::smallint IS NULL OR u.rs_score <= $8)
      AND (($9::text IS NULL AND $10::text IS NULL) OR EXISTS (
          SELECT 1 FROM daily_technical_signal t
          WHERE t.stock_symbol = u.stock_symbol
            AND t.date BETWEEN u.window_start AND u.date
            AND ($9::text IS NULL OR t.indicator = $9)
            AND ($10::text IS NULL OR t.kind = $10)
      ))"#,
            $chip_filter,
            r#"
),
chosen AS (
    SELECT date, COUNT(*) AS holdings, AVG(ret)::float8 AS avg_ret,
           array_agg(stock_symbol::text ORDER BY pick_rank) AS symbols
    FROM picks
    WHERE pick_rank <= $5
    GROUP BY date
)
SELECT b.date,
       COALESCE(c.holdings, 0) AS holdings,
       c.avg_ret AS avg_return,
       b.avg_ret AS benchmark_return,
       COALESCE(c.symbols, ARRAY[]::text[]) AS symbols
FROM bench b
LEFT JOIN chosen c ON c.date = b.date
WHERE b.samples > 0
ORDER BY b.date
"#
        )
    };
}

/// 沒有籌碼條件的回測。
const BACKTEST_SQL: &str = backtest_sql!("\nWITH", "", "");

/// 有籌碼條件的回測：換股當天外資（`$11`）／投信（`$12`）已連續買超至少 N 天，`NULL` 為不篩選。
const BACKTEST_WITH_CHIP_SQL: &str = concat!(
    "\nWITH",
    chip_streak_ctes!(),
    ",",
    backtest_sql!(
        "",
        "\n    JOIN chip c ON c.\"date\" = u.date AND c.stock_symbol = u.stock_symbol",
        "\n      AND ($11::bigint IS NULL OR c.foreign_buy >= $11)\n      AND ($12::bigint IS NULL OR c.trust_buy >= $12)"
    )
);

/// [`BACKTEST_SQL`] 的一列：一期的結果；`symbols` 是全部持股，依相對強度由高到低。
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
struct PeriodRow {
    date: NaiveDate,
    holdings: i64,
    avg_return: Option<f64>,
    benchmark_return: Option<f64>,
    symbols: Vec<String>,
}

/// 依條件每隔 `hold_days` 個交易日換股，回測等權持有前 `limit` 檔（相對強度由高到低）的報酬，
/// 並與同市場全部母體股票的平均比較。
///
/// # Errors
///
/// 日期格式錯誤、起日晚於迄日、期間超過 3,653 天、`hold_days` 不是 5／20／60／120／240、`limit`
/// 不在 1–100、`fee_discount` 不在 0–1、相對強度或連續買超天數超出範圍、訊號來源／種類不在固定清單
/// 或產業名稱不存在回 422；
/// 迄日之前沒有相對強度資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest", tag = "data-api", params(ScreenBacktestParams), responses((status = 200, body = ScreenBacktestResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest(Query(params): Query<ScreenBacktestParams>) -> Response {
    let (from, to) = match (
        parse_optional_date(params.from.as_deref()),
        parse_optional_date(params.to.as_deref()),
    ) {
        (Ok(from), Ok(to)) => (from, to),
        (Err(message), _) | (_, Err(message)) => {
            return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
        }
    };
    let hold_days = params.hold_days.unwrap_or(20);
    if !HOLD_DAYS.contains(&hold_days) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "hold_days 必須為 5、20、60、120 或 240",
        );
    }
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 100");
    }
    let fee_discount = params.fee_discount.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&fee_discount) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "fee_discount 必須介於 0 至 1",
        );
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    if let Some(response) = signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
    {
        return response;
    }
    if let Err(message) = validate_rs_range(params.min_rs_score, params.max_rs_score) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    if let Err(message) = validate_streaks(params.min_foreign_streak, params.min_trust_streak) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }

    let pool = database::get_connection();
    let latest: Option<NaiveDate> = match sqlx::query_scalar(LATEST_RS_DATE_SQL)
        .bind(to)
        .fetch_one(pool)
        .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let Some(to) = latest else {
        return error_response(StatusCode::NOT_FOUND, "查無相對強度資料");
    };
    let from = match resolve_backtest_period(from, to) {
        Ok(from) => from,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    let with_chip = params.min_foreign_streak.is_some() || params.min_trust_streak.is_some();
    let mut query = sqlx::query_as(if with_chip {
        BACKTEST_WITH_CHIP_SQL
    } else {
        BACKTEST_SQL
    })
    .bind(from)
    .bind(to)
    .bind(market_id)
    .bind(i32::from(hold_days))
    .bind(i64::from(limit))
    .bind(params.industry.as_deref())
    .bind(params.min_rs_score.map(i16::from))
    .bind(params.max_rs_score.map(i16::from))
    .bind(params.indicator.as_deref())
    .bind(params.kind.as_deref());
    if with_chip {
        query = query
            .bind(params.min_foreign_streak.map(i64::from))
            .bind(params.min_trust_streak.map(i64::from));
    }
    let rows: Vec<PeriodRow> = match query.fetch_all(pool).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let costs = trading_costs(&rows, fee_discount);

    Json(ScreenBacktestResponse {
        from: from.to_string(),
        to: to.to_string(),
        market: market.to_owned(),
        hold_days,
        limit,
        industry: params.industry.clone(),
        min_rs_score: params.min_rs_score,
        max_rs_score: params.max_rs_score,
        min_foreign_streak: params.min_foreign_streak,
        min_trust_streak: params.min_trust_streak,
        indicator: params.indicator.clone(),
        kind: params.kind.clone(),
        fee_discount,
        adjusted: true,
        summary: summarize(&rows, &costs, hold_days),
        periods: rows.into_iter().zip(costs).map(period).collect(),
    })
    .into_response()
}

/// 起日：未指定為迄日往前 [`DEFAULT_PERIOD_DAYS`] 天；晚於迄日或期間超過上限回錯誤訊息。
fn resolve_backtest_period(
    from: Option<NaiveDate>,
    to: NaiveDate,
) -> Result<NaiveDate, &'static str> {
    let from = from.unwrap_or(to - chrono::Duration::days(DEFAULT_PERIOD_DAYS));
    if from > to {
        return Err("from 不可晚於 to");
    }
    if (to - from).num_days() >= MAX_PERIOD_DAYS {
        return Err("回測期間最長 3,653 天");
    }
    Ok(from)
}

/// 一期的換手與扣成本後報酬。
#[derive(Debug, Clone, Copy, PartialEq)]
struct PeriodCost {
    /// 換股時新買進的部位比例（0–1）；空手為 0。
    turnover: f64,
    /// 扣除這次換股成本後的報酬（%）。
    net_return: f64,
}

/// 依前後兩期的持股算出每期的換手與扣成本後報酬：賣出上一期有、這一期沒有的股票（占上一期的比例），
/// 付手續費與證交稅；買進這一期新增的股票（占這一期的比例），付手續費。第一期全部買進。
fn trading_costs(rows: &[PeriodRow], fee_discount: f64) -> Vec<PeriodCost> {
    let fee = FEE_RATE * fee_discount;
    let mut previous: &[String] = &[];
    rows.iter()
        .map(|row| {
            let current = row.symbols.as_slice();
            let share = |from: &[String], other: &[String]| {
                if from.is_empty() {
                    0.0
                } else {
                    from.iter().filter(|symbol| !other.contains(symbol)).count() as f64
                        / from.len() as f64
                }
            };
            let sold = share(previous, current);
            let bought = share(current, previous);
            previous = current;
            PeriodCost {
                turnover: bought,
                net_return: row.avg_return.unwrap_or(0.0) - sold * (fee + TAX_RATE) - bought * fee,
            }
        })
        .collect()
}

/// 一期的輸出：報酬四捨五入到小數兩位；代號只列前 [`LISTED_SYMBOLS`] 檔。
fn period((mut row, cost): (PeriodRow, PeriodCost)) -> BacktestPeriod {
    row.symbols.truncate(LISTED_SYMBOLS);
    BacktestPeriod {
        date: row.date.to_string(),
        holdings: row.holdings,
        avg_return: row.avg_return.map(round2),
        return_after_costs: round2(cost.net_return),
        turnover: round4(cost.turnover),
        benchmark_return: row.benchmark_return.map(round2),
        symbols: row.symbols,
    }
}

/// 累積淨值（起始 1）走過各期報酬（%）後的最終淨值與最大回撤（比例，負值或 0）。
fn compound(returns: impl IntoIterator<Item = f64>) -> (f64, f64) {
    let mut equity = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut drawdown = 0.0_f64;
    for percent in returns {
        equity *= 1.0 + percent / 100.0;
        peak = peak.max(equity);
        drawdown = drawdown.min(equity / peak - 1.0);
    }
    (equity, drawdown)
}

/// 彙總各期：空手的期數報酬算 0；年化以每年約 [`TRADING_DAYS_PER_YEAR`] 個交易日換算；
/// `costs` 與 `rows` 一一對應（見 [`trading_costs`]）。
fn summarize(rows: &[PeriodRow], costs: &[PeriodCost], hold_days: u16) -> BacktestSummary {
    let periods = rows.len();
    let invested: Vec<&PeriodRow> = rows.iter().filter(|row| row.holdings > 0).collect();
    let returns: Vec<f64> = rows
        .iter()
        .map(|row| row.avg_return.unwrap_or(0.0))
        .collect();
    let benchmarks: Vec<f64> = rows
        .iter()
        .map(|row| row.benchmark_return.unwrap_or(0.0))
        .collect();
    if periods == 0 {
        return BacktestSummary {
            periods: 0,
            invested_periods: 0,
            avg_holdings: None,
            avg_turnover: None,
            total_return: None,
            total_return_after_costs: None,
            benchmark_total_return: None,
            annualized_return: None,
            annualized_return_after_costs: None,
            benchmark_annualized_return: None,
            max_drawdown: None,
            max_drawdown_after_costs: None,
            benchmark_max_drawdown: None,
            win_rate: None,
            beat_market_rate: None,
            avg_period_return: None,
            avg_benchmark_return: None,
        };
    }
    let count = periods as f64;
    let years = count * f64::from(hold_days) / TRADING_DAYS_PER_YEAR;
    let annualize = |equity: f64| (equity.powf(1.0 / years) - 1.0) * 100.0;
    let (equity, drawdown) = compound(returns.iter().copied());
    let (benchmark_equity, benchmark_drawdown) = compound(benchmarks.iter().copied());
    let (net_equity, net_drawdown) = compound(costs.iter().map(|cost| cost.net_return));
    let invested_turnover: Vec<f64> = rows
        .iter()
        .zip(costs)
        .filter(|(row, _)| row.holdings > 0)
        .map(|(_, cost)| cost.turnover)
        .collect();
    let ratio = |hits: usize| round4(hits as f64 / count);
    BacktestSummary {
        periods: periods as i64,
        invested_periods: invested.len() as i64,
        avg_holdings: (!invested.is_empty()).then(|| {
            round2(
                invested.iter().map(|row| row.holdings as f64).sum::<f64>() / invested.len() as f64,
            )
        }),
        avg_turnover: (!invested_turnover.is_empty()).then(|| {
            round4(invested_turnover.iter().sum::<f64>() / invested_turnover.len() as f64)
        }),
        total_return: Some(round2((equity - 1.0) * 100.0)),
        total_return_after_costs: Some(round2((net_equity - 1.0) * 100.0)),
        benchmark_total_return: Some(round2((benchmark_equity - 1.0) * 100.0)),
        annualized_return: Some(round2(annualize(equity))),
        annualized_return_after_costs: Some(round2(annualize(net_equity))),
        benchmark_annualized_return: Some(round2(annualize(benchmark_equity))),
        max_drawdown: Some(round2(drawdown * 100.0)),
        max_drawdown_after_costs: Some(round2(net_drawdown * 100.0)),
        benchmark_max_drawdown: Some(round2(benchmark_drawdown * 100.0)),
        win_rate: Some(ratio(returns.iter().filter(|r| **r > 0.0).count())),
        beat_market_rate: Some(ratio(
            returns
                .iter()
                .zip(&benchmarks)
                .filter(|(r, b)| r > b)
                .count(),
        )),
        avg_period_return: Some(round2(returns.iter().sum::<f64>() / count)),
        avg_benchmark_return: Some(round2(benchmarks.iter().sum::<f64>() / count)),
    }
}

#[cfg(test)]
mod tests;
