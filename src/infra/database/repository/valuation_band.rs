//! 估值區間每日倍數的 PostgreSQL 實作（[`ValuationHistoryRepository`]）。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    domain::quote::valuation_band::{DailyRatio, ValuationHistoryRepository},
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// 期間內每日的倍數，依日期由新到舊；走 `(stock_symbol, "Date" DESC)` 的覆蓋索引。
/// 停牌等沒有成交（收盤價 0）的占位列不算，避免被當成最新一日。
const DAILY_RATIOS_SQL: &str = r#"
SELECT "Date", "ClosingPrice", "PriceEarningRatio", "price-to-book_ratio"
FROM "DailyQuotes"
WHERE stock_symbol = $1
  AND "ClosingPrice" > 0
  AND "Date" > (
      SELECT max("Date") FROM "DailyQuotes" WHERE stock_symbol = $1 AND "ClosingPrice" > 0
  ) - make_interval(years => $2)
ORDER BY "Date" DESC
"#;

#[async_trait]
impl ValuationHistoryRepository for PgQuoteRepository {
    async fn fetch_daily_ratios(&self, stock_symbol: &str, years: u8) -> Result<Vec<DailyRatio>> {
        let rows: Vec<(NaiveDate, Decimal, Decimal, Decimal)> = sqlx::query_as(DAILY_RATIOS_SQL)
            .bind(stock_symbol)
            .bind(i32::from(years))
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(date, close, price_earning_ratio, price_to_book_ratio)| DailyRatio {
                    date,
                    close,
                    price_earning_ratio,
                    price_to_book_ratio,
                },
            )
            .collect())
    }
}
