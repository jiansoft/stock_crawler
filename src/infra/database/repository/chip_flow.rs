//! 法人買賣超（[`ChipFlowQuery`]）的 PostgreSQL 實作，讀 `chip_daily`。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::chip::flow::{ChipFlowQuery, FlowPeriod, STREAK_WINDOW, StockFlow, StockStreak},
    infra::database::{self, repository::chip::PgChipRepository},
};

/// `$1`（含）之前最近 [`STREAK_WINDOW`] 個有籌碼資料的交易日，由新到舊。
///
/// 只回看 200 個日曆天，足以涵蓋 60 個交易日（含春節等長假）。
const DATES_SQL: &str = r#"
SELECT DISTINCT "date"
FROM chip_daily
WHERE "date" <= $1 AND "date" > $1 - 200
ORDER BY 1 DESC
LIMIT $2
"#;

/// 上市櫃股票在 `$2`～`$1` 期間的法人（`$4`）淨買賣，與截至 `$1` 的連續天數（回看到 `$3`）。
///
/// - `total` 為三者相加，任一為 NULL（來源沒有這檔）即為 NULL。
/// - 連續天數：資料日的買賣方向往回數，遇到方向不同或沒有買賣的一天即停；資料日沒有買賣為 0。
/// - 金額以當日收盤價估算；沒有成交的日子不計金額、成交量與成交金額。
const FLOW_SQL: &str = r#"
WITH net AS (
    SELECT stock_symbol, "date",
           CASE $4
               WHEN 'foreign' THEN foreign_net
               WHEN 'trust' THEN trust_net
               WHEN 'dealer' THEN dealer_net
               ELSE foreign_net + trust_net + dealer_net
           END AS net
    FROM chip_daily
    WHERE "date" BETWEEN $3 AND $1
),
latest AS (
    SELECT stock_symbol, SIGN(net)::int AS sign
    FROM net
    WHERE "date" = $1 AND net <> 0
),
breaks AS (
    SELECT l.stock_symbol, l.sign,
           MAX(n."date") FILTER (WHERE n.net IS NULL OR SIGN(n.net)::int <> l.sign) AS break_date
    FROM latest l
    JOIN net n ON n.stock_symbol = l.stock_symbol
    GROUP BY l.stock_symbol, l.sign
),
streaks AS (
    SELECT b.stock_symbol, (b.sign * COUNT(*))::int AS streak
    FROM breaks b
    JOIN net n ON n.stock_symbol = b.stock_symbol
              AND n."date" > COALESCE(b.break_date, '-infinity'::date)
    GROUP BY b.stock_symbol, b.sign
),
period AS (
    SELECT n.stock_symbol,
           SUM(n.net)::bigint AS net_shares,
           COALESCE(SUM(n.net * q."ClosingPrice"), 0)::float8 AS net_amount,
           SUM(q."TradingVolume")::float8 AS volume,
           SUM(q."TradeValue")::float8 AS trade_value,
           COUNT(*) FILTER (WHERE n.net > 0) AS buy_days,
           COUNT(*) FILTER (WHERE n.net < 0) AS sell_days
    FROM net n
    LEFT JOIN "DailyQuotes" q
           ON q.stock_symbol = n.stock_symbol AND q."Date" = n."date" AND q."ClosingPrice" > 0
    WHERE n."date" >= $2 AND n.net IS NOT NULL
    GROUP BY n.stock_symbol
)
SELECT p.stock_symbol,
       s."Name" AS name,
       s.stock_exchange_market_id AS market_id,
       COALESCE(si.name, '') AS industry,
       p.net_shares,
       p.net_amount,
       p.volume,
       p.trade_value,
       p.buy_days,
       p.sell_days,
       COALESCE(st.streak, 0) AS streak,
       dq."ClosingPrice"::float8 AS closing_price
FROM period p
JOIN stocks s ON s.stock_symbol = p.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN streaks st ON st.stock_symbol = p.stock_symbol
LEFT JOIN "DailyQuotes" dq
       ON dq.stock_symbol = p.stock_symbol AND dq."Date" = $1 AND dq."ClosingPrice" > 0
WHERE s.stock_exchange_market_id IN (2, 4)
"#;

/// 各股票截至 `$1` 的外資、投信連續買賣超天數（回看到 `$2`）。
///
/// 與 [`FLOW_SQL`] 同一算法，只算外資與投信的連續天數；資料日沒有買賣為 0。
const STREAKS_SQL: &str = r#"
WITH net AS (
    SELECT stock_symbol, "date", foreign_net, trust_net
    FROM chip_daily
    WHERE "date" BETWEEN $2 AND $1
),
latest AS (
    SELECT stock_symbol,
           COALESCE(SIGN(foreign_net)::int, 0) AS foreign_sign,
           COALESCE(SIGN(trust_net)::int, 0) AS trust_sign
    FROM net
    WHERE "date" = $1
),
breaks AS (
    SELECT l.stock_symbol, l.foreign_sign, l.trust_sign,
           COALESCE(MAX(n."date") FILTER (
               WHERE n.foreign_net IS NULL OR SIGN(n.foreign_net)::int <> l.foreign_sign
           ), '-infinity'::date) AS foreign_break,
           COALESCE(MAX(n."date") FILTER (
               WHERE n.trust_net IS NULL OR SIGN(n.trust_net)::int <> l.trust_sign
           ), '-infinity'::date) AS trust_break
    FROM latest l
    JOIN net n ON n.stock_symbol = l.stock_symbol
    GROUP BY l.stock_symbol, l.foreign_sign, l.trust_sign
)
SELECT b.stock_symbol,
       (b.foreign_sign * COUNT(*) FILTER (WHERE n."date" > b.foreign_break))::smallint
           AS foreign_streak,
       (b.trust_sign * COUNT(*) FILTER (WHERE n."date" > b.trust_break))::smallint
           AS trust_streak
FROM breaks b
JOIN net n ON n.stock_symbol = b.stock_symbol
GROUP BY b.stock_symbol, b.foreign_sign, b.trust_sign
"#;

/// [`FLOW_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct FlowRow {
    stock_symbol: String,
    name: String,
    market_id: i32,
    industry: String,
    net_shares: i64,
    net_amount: f64,
    volume: Option<f64>,
    trade_value: Option<f64>,
    buy_days: i64,
    sell_days: i64,
    streak: i32,
    closing_price: Option<f64>,
}

#[async_trait]
impl ChipFlowQuery for PgChipRepository {
    async fn fetch_chip_dates(&self, until: NaiveDate) -> Result<Vec<NaiveDate>> {
        Ok(sqlx::query_scalar(DATES_SQL)
            .bind(until)
            .bind(STREAK_WINDOW as i64)
            .fetch_all(database::get_connection())
            .await?)
    }

    async fn fetch_flows(
        &self,
        period: FlowPeriod,
        window_start: NaiveDate,
        investor: &str,
    ) -> Result<Vec<StockFlow>> {
        let rows: Vec<FlowRow> = sqlx::query_as(FLOW_SQL)
            .bind(period.data_date)
            .bind(period.from)
            .bind(window_start)
            .bind(investor)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| StockFlow {
                stock_symbol: row.stock_symbol,
                name: row.name,
                market_id: row.market_id,
                industry: row.industry,
                net_shares: row.net_shares,
                net_amount: row.net_amount,
                volume: row.volume,
                trade_value: row.trade_value,
                buy_days: row.buy_days,
                sell_days: row.sell_days,
                streak: row.streak,
                closing_price: row.closing_price,
            })
            .collect())
    }

    async fn fetch_streaks(
        &self,
        data_date: NaiveDate,
        window_start: NaiveDate,
    ) -> Result<Vec<StockStreak>> {
        let rows: Vec<(String, i16, i16)> = sqlx::query_as(STREAKS_SQL)
            .bind(data_date)
            .bind(window_start)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(stock_symbol, foreign_streak, trust_streak)| StockStreak {
                stock_symbol,
                foreign_streak,
                trust_streak,
            })
            .collect())
    }
}
