//! 回測前後的輔助查詢：最新相對強度日、集保最早資料週、續抱持股的平均成交金額。

/// `$1`（含）之前最近一個有相對強度的交易日；`$1` 為 `NULL` 時取最新。
pub(in super::super) const LATEST_RS_DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_relative_strength WHERE $1::date IS NULL OR date <= $1
"#;

/// 集保股權分散最早的資料週；設 `min_major_change` 時用來檢查起日。
pub(in super::super) const FIRST_HOLDER_WEEK_SQL: &str = r#"
SELECT MIN("date") FROM holder_distribution
"#;

/// 續抱持股的平均成交金額：`$1`／`$2`／`$3` 為一一對應的換股日、代號與 `liquidity_start`，回傳
/// `liquidity_start`～換股日的平均成交金額（與 `valued` 的算法相同；沒有日線為 `NULL`）。
pub(in super::super) const HOLD_TRADE_VALUES_SQL: &str = r#"
SELECT k.date, k.stock_symbol, (
    SELECT AVG(q."TradeValue")::float8 FROM "DailyQuotes" q
    WHERE q.stock_symbol = k.stock_symbol AND q."Date" BETWEEN k.liquidity_start AND k.date
) AS trade_value
FROM UNNEST($1::date[], $2::text[], $3::date[]) AS k(date, stock_symbol, liquidity_start)
"#;
