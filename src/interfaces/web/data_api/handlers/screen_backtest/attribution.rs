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

use std::collections::HashMap;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

use super::preset::BacktestQuery;
use super::{Backtest, PeriodRow};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, ScreenBacktestAttributionResponse, ScreenBacktestParams,
};
use crate::interfaces::web::data_api::handlers::{database_error, error_response};

/// 基準的檔數。
pub(super) const BENCHMARK_SIZE: i64 = 50;

/// 基準先取換股日當天成交金額前幾名，再依 20 日平均成交金額取前 [`BENCHMARK_SIZE`] 檔。
pub(super) const PRESELECT_SIZE: i64 = 300;

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
mod brinson;

use self::brinson::{
    BenchmarkRow, Bucket, PeriodAttribution, attribute, attribution_years, industry_effects, totals,
};

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
        Ok(backtest) => backtest.period_end_only(),
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
mod tests;
