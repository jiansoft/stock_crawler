//! 法人買賣超（[`ChipFlowQuery`]）的 PostgreSQL 實作，讀 `chip_daily`。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::chip::flow::{ChipFlowQuery, FlowPeriod, MAX_FLOW_DAYS, StockFlow, StockStreak},
    infra::database::{self, repository::chip::PgChipRepository},
};

/// `$1`（含）之前最近 [`MAX_FLOW_DAYS`] 個有籌碼資料的交易日，由新到舊（累計期間最多這麼長）。
///
/// 只回看 200 個日曆天，足以涵蓋 60 個交易日（含春節等長假）。
const DATES_SQL: &str = r#"
SELECT DISTINCT "date"
FROM chip_daily
WHERE "date" <= $1 AND "date" > $1 - 200
ORDER BY 1 DESC
LIMIT $2
"#;

/// 上市櫃股票在 `$2`～`$1` 期間的法人（`$3`）淨買賣，與資料日 `$1` 的連續天數。
///
/// - `total` 為三者相加，任一為 NULL（來源沒有這檔）即為 NULL，不列入期間累計。
/// - 連續天數讀 `chip_daily` 預先算好的欄位（見 `PgChipRepository::refresh_streaks`）；
///   資料日沒有買賣或尚未計算為 0。
/// - 金額以當日收盤價估算；沒有成交的日子不計金額、成交量與成交金額。
const FLOW_SQL: &str = r#"
WITH net AS (
    SELECT stock_symbol, "date",
           CASE $3
               WHEN 'foreign' THEN foreign_net
               WHEN 'trust' THEN trust_net
               WHEN 'dealer' THEN dealer_net
               ELSE foreign_net + trust_net + dealer_net
           END AS net
    FROM chip_daily
    WHERE "date" BETWEEN $2 AND $1
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
    WHERE n.net IS NOT NULL
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
       COALESCE(CASE $3
                    WHEN 'foreign' THEN cd.foreign_streak
                    WHEN 'trust' THEN cd.trust_streak
                    WHEN 'dealer' THEN cd.dealer_streak
                    ELSE cd.total_streak
                END, 0)::int AS streak,
       dq."ClosingPrice"::float8 AS closing_price
FROM period p
JOIN stocks s ON s.stock_symbol = p.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN chip_daily cd ON cd."date" = $1 AND cd.stock_symbol = p.stock_symbol
LEFT JOIN "DailyQuotes" dq
       ON dq.stock_symbol = p.stock_symbol AND dq."Date" = $1 AND dq."ClosingPrice" > 0
WHERE s.stock_exchange_market_id IN (2, 4)
"#;

/// 各股票在資料日 `$1` 的外資、投信連續買賣超天數（預先算好的欄位）；尚未計算為 0。
const STREAKS_SQL: &str = r#"
SELECT stock_symbol,
       COALESCE(foreign_streak, 0)::smallint AS foreign_streak,
       COALESCE(trust_streak, 0)::smallint AS trust_streak
FROM chip_daily
WHERE "date" = $1
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
            .bind(MAX_FLOW_DAYS as i64)
            .fetch_all(database::get_connection())
            .await?)
    }

    async fn fetch_flows(&self, period: FlowPeriod, investor: &str) -> Result<Vec<StockFlow>> {
        let rows: Vec<FlowRow> = sqlx::query_as(FLOW_SQL)
            .bind(period.data_date)
            .bind(period.from)
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

    async fn fetch_streaks(&self, data_date: NaiveDate) -> Result<Vec<StockStreak>> {
        let rows: Vec<(String, i16, i16)> = sqlx::query_as(STREAKS_SQL)
            .bind(data_date)
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
