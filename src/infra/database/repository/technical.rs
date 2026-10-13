//! 技術指標日線的 PostgreSQL 實作（[`PriceBarRepository`]）。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    domain::quote::technical::{PriceBar, PriceBarRepository},
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// 截止日（含）之前最近 `$3` 個有成交的交易日，由新到舊；只讀
/// `(stock_symbol, "Date" DESC)` 覆蓋索引內的欄位（index-only scan）。
const PRICE_BARS_SQL: &str = r#"
SELECT "Date", "ClosingPrice", "HighestPrice", "LowestPrice"
FROM "DailyQuotes"
WHERE stock_symbol = $1
  AND "ClosingPrice" > 0
  AND ($2::date IS NULL OR "Date" <= $2)
ORDER BY "Date" DESC
LIMIT $3
"#;

#[async_trait]
impl PriceBarRepository for PgQuoteRepository {
    async fn fetch_price_bars(
        &self,
        stock_symbol: &str,
        until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<PriceBar>> {
        let rows: Vec<(NaiveDate, Decimal, Decimal, Decimal)> = sqlx::query_as(PRICE_BARS_SQL)
            .bind(stock_symbol)
            .bind(until)
            .bind(limit)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .rev()
            .map(|(date, close, high, low)| PriceBar {
                date,
                close: close.to_f64().unwrap_or_default(),
                high: high.to_f64().unwrap_or_default(),
                low: low.to_f64().unwrap_or_default(),
            })
            .collect())
    }
}
