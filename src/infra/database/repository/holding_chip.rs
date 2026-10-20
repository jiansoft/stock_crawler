//! 持股籌碼日報的 PostgreSQL 查詢：查詢日換成交易日，以及某交易日收盤時的持股與當天籌碼。
//!
//! 介面與持股推算規則見 [`crate::domain::portfolio::chip::HoldingChipQuery`]；stock_go 的持股籌碼頁用的是
//! 同一段 SQL。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};
use sqlx::types::Json;

use super::portfolio::PgPortfolioRepository;
use crate::domain::chip::BrokerNetRecord;
use crate::domain::portfolio::chip::{
    ChipTradingDay, HoldingBroker, HoldingChip, HoldingChipQuery,
};
use crate::infra::database;

/// `$1` 查詢日（`NULL` 為今天）：不晚於它的最近交易日與前後一個交易日（以 `chip_daily` 為準）。
const RESOLVE_DAY_SQL: &str = r#"
WITH d AS (
    SELECT MAX("date") AS d FROM chip_daily WHERE "date" <= COALESCE($1::date, CURRENT_DATE)
)
SELECT d.d,
       (SELECT MAX("date") FROM chip_daily WHERE "date" < d.d),
       (SELECT MIN("date") FROM chip_daily WHERE "date" > d.d)
FROM d
"#;

/// `$1` 交易日：當天收盤時仍持有的普通股（只有數字、不是 00 開頭）與當天籌碼。
///
/// 已賣出批次的 `sell_history_serial` 多半沒有回填，賣出日取同會員、同代號、買進日之後最早一筆
/// 未撤銷的賣出紀錄；找不到時視為買進當天就不再持有。
const HOLDING_CHIP_SQL: &str = r#"
WITH held AS (
    SELECT DISTINCT s.security_code AS stock_symbol
    FROM stock_ownership_details s
    WHERE s.date <= $1::date
      AND s.security_code ~ '^[0-9]+$' AND s.security_code NOT LIKE '00%'
      AND (NOT s.is_sold OR COALESCE((
            SELECT MIN(h.date) FROM sell_history h
            WHERE h.member_id = s.member_id AND h.security_code = s.security_code
              AND h.is_reverted IS NOT TRUE AND h.date >= s.date), s.date) > $1::date)
)
SELECT held.stock_symbol, COALESCE(st."Name", '') AS name,
       q."ClosingPrice" AS closing_price, q."TradingVolume" AS trading_volume,
       c.foreign_net, c.trust_net, c.dealer_net, c.foreign_streak, c.trust_streak,
       c.margin_previous, c.margin_balance, c.short_previous, c.short_balance,
       b.buy_total, b.sell_total, b.main_share, b.buyers, b.sellers
FROM held
LEFT JOIN stocks st ON st.stock_symbol = held.stock_symbol
LEFT JOIN chip_daily c ON c.stock_symbol = held.stock_symbol AND c."date" = $1::date
LEFT JOIN "DailyQuotes" q ON q.stock_symbol = held.stock_symbol AND q."Date" = $1::date
LEFT JOIN broker_flow b ON b.stock_symbol = held.stock_symbol AND b."date" = $1::date
ORDER BY held.stock_symbol
"#;

/// [`HOLDING_CHIP_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct HoldingChipRow {
    stock_symbol: String,
    name: String,
    closing_price: Option<Decimal>,
    trading_volume: Option<Decimal>,
    foreign_net: Option<i64>,
    trust_net: Option<i64>,
    dealer_net: Option<i64>,
    foreign_streak: Option<i16>,
    trust_streak: Option<i16>,
    margin_previous: Option<i64>,
    margin_balance: Option<i64>,
    short_previous: Option<i64>,
    short_balance: Option<i64>,
    buy_total: Option<i64>,
    sell_total: Option<i64>,
    main_share: Option<Decimal>,
    buyers: Option<Json<Vec<BrokerNetRecord>>>,
    sellers: Option<Json<Vec<BrokerNetRecord>>>,
}

impl From<HoldingChipRow> for HoldingChip {
    fn from(row: HoldingChipRow) -> Self {
        let broker = match (row.buy_total, row.sell_total, row.main_share) {
            (Some(buy_total), Some(sell_total), Some(main_share)) => Some(HoldingBroker {
                buy_total,
                sell_total,
                main_share,
                buyers: row.buyers.map(|json| json.0).unwrap_or_default(),
                sellers: row.sellers.map(|json| json.0).unwrap_or_default(),
            }),
            _ => None,
        };
        HoldingChip {
            stock_symbol: row.stock_symbol,
            name: row.name,
            closing_price: row.closing_price,
            volume: row
                .trading_volume
                .and_then(|volume| volume.round().to_i64()),
            foreign_net: row.foreign_net,
            trust_net: row.trust_net,
            dealer_net: row.dealer_net,
            foreign_streak: row.foreign_streak.map(i32::from),
            trust_streak: row.trust_streak.map(i32::from),
            margin_previous: row.margin_previous,
            margin_balance: row.margin_balance,
            short_previous: row.short_previous,
            short_balance: row.short_balance,
            broker,
        }
    }
}

#[async_trait]
impl HoldingChipQuery for PgPortfolioRepository {
    async fn resolve_chip_day(&self, date: Option<NaiveDate>) -> Result<Option<ChipTradingDay>> {
        let row: (Option<NaiveDate>, Option<NaiveDate>, Option<NaiveDate>) =
            sqlx::query_as(RESOLVE_DAY_SQL)
                .bind(date)
                .fetch_one(database::get_connection())
                .await
                .context("Failed to resolve chip trading day")?;
        Ok(row.0.map(|date| ChipTradingDay {
            date,
            previous: row.1,
            next: row.2,
        }))
    }

    async fn fetch_holding_chips(&self, date: NaiveDate) -> Result<Vec<HoldingChip>> {
        let rows: Vec<HoldingChipRow> = sqlx::query_as(HOLDING_CHIP_SQL)
            .bind(date)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch holding chips")?;
        Ok(rows.into_iter().map(HoldingChip::from).collect())
    }
}

#[cfg(test)]
mod tests;
