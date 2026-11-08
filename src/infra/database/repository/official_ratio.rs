//! 官方本益比／股價淨值比回補的 PostgreSQL 實作（[`OfficialRatioRepository`]）。
//!
//! 只寫 `DailyQuotes` 的 `"PriceEarningRatio"` 與 `"price-to-book_ratio"`。淨值比只填 0；
//! 本益比依 [`PePolicy`]：上市只填 0，上櫃以官方值為準覆寫（原因見領域模組說明）。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    domain::quote::official_ratio::{OfficialRatio, OfficialRatioRepository, PePolicy},
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// 仍有缺值的交易日：只看有成交的非 ETF 個股，走 `"Date"` 索引再以主檔過濾市場。
const MISSING_DATES_SQL: &str = r#"
SELECT DISTINCT q."Date"
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $2 AND $3
  AND s.stock_exchange_market_id = $1
  AND q."ClosingPrice" > 0
  AND q.stock_symbol !~ '^00'
  AND (q."PriceEarningRatio" = 0 OR q."price-to-book_ratio" = 0)
ORDER BY q."Date"
"#;

/// 指定市場所有有成交的交易日（[`PePolicy::Authoritative`] 用）。
const ALL_DATES_SQL: &str = r#"
SELECT DISTINCT q."Date"
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $2 AND $3
  AND s.stock_exchange_market_id = $1
  AND q."ClosingPrice" > 0
  AND q.stock_symbol !~ '^00'
ORDER BY q."Date"
"#;

/// 只填目前為 0 且官方有值的欄位；兩欄都不需要更新的列不碰（不產生無謂的 row version）。
const FILL_SQL: &str = r#"
UPDATE "DailyQuotes" q SET
    "PriceEarningRatio" = CASE WHEN q."PriceEarningRatio" = 0 AND r.pe IS NOT NULL
        THEN r.pe ELSE q."PriceEarningRatio" END,
    "price-to-book_ratio" = CASE WHEN q."price-to-book_ratio" = 0 AND r.pb IS NOT NULL
        THEN r.pb ELSE q."price-to-book_ratio" END
FROM UNNEST($2::varchar[], $3::numeric[], $4::numeric[]) AS r(symbol, pe, pb)
WHERE q."Date" = $1
  AND q.stock_symbol = r.symbol
  AND ((q."PriceEarningRatio" = 0 AND r.pe IS NOT NULL)
    OR (q."price-to-book_ratio" = 0 AND r.pb IS NOT NULL))
"#;

/// 本益比以官方值為準（官方沒有值寫 0）、淨值比只填 0；值已相同的列不碰。
const AUTHORITATIVE_FILL_SQL: &str = r#"
UPDATE "DailyQuotes" q SET
    "PriceEarningRatio" = COALESCE(r.pe, 0),
    "price-to-book_ratio" = CASE WHEN q."price-to-book_ratio" = 0 AND r.pb IS NOT NULL
        THEN r.pb ELSE q."price-to-book_ratio" END
FROM UNNEST($2::varchar[], $3::numeric[], $4::numeric[]) AS r(symbol, pe, pb)
WHERE q."Date" = $1
  AND q.stock_symbol = r.symbol
  AND (q."PriceEarningRatio" IS DISTINCT FROM COALESCE(r.pe, 0)
    OR (q."price-to-book_ratio" = 0 AND r.pb IS NOT NULL))
"#;

/// 最新報價表與同日日線的倍數不同時改成日線的值；值相同的列不碰。
const SYNC_LATEST_SQL: &str = r#"
UPDATE last_daily_quotes l SET
    price_earning_ratio = q."PriceEarningRatio",
    "price-to-book_ratio" = q."price-to-book_ratio",
    updated_time = now()
FROM "DailyQuotes" q
WHERE q."Date" = l.date
  AND q.stock_symbol = l.stock_symbol
  AND (l.price_earning_ratio IS DISTINCT FROM q."PriceEarningRatio"
    OR l."price-to-book_ratio" IS DISTINCT FROM q."price-to-book_ratio")
"#;

#[async_trait]
impl OfficialRatioRepository for PgQuoteRepository {
    async fn fetch_dates_to_update(
        &self,
        market_id: i32,
        from: NaiveDate,
        to: NaiveDate,
        policy: PePolicy,
    ) -> Result<Vec<NaiveDate>> {
        let sql = match policy {
            PePolicy::FillZeroOnly => MISSING_DATES_SQL,
            PePolicy::Authoritative => ALL_DATES_SQL,
        };
        Ok(sqlx::query_scalar(sql)
            .bind(market_id)
            .bind(from)
            .bind(to)
            .fetch_all(database::get_connection())
            .await?)
    }

    async fn fill_official_ratios(
        &self,
        date: NaiveDate,
        ratios: &[OfficialRatio],
        policy: PePolicy,
    ) -> Result<u64> {
        if ratios.is_empty() {
            return Ok(0);
        }
        let symbols: Vec<String> = ratios.iter().map(|r| r.stock_symbol.clone()).collect();
        let pe: Vec<Option<Decimal>> = ratios.iter().map(|r| r.price_earning_ratio).collect();
        let pb: Vec<Option<Decimal>> = ratios.iter().map(|r| r.price_to_book_ratio).collect();
        let sql = match policy {
            PePolicy::FillZeroOnly => FILL_SQL,
            PePolicy::Authoritative => AUTHORITATIVE_FILL_SQL,
        };
        let result = sqlx::query(sql)
            .bind(date)
            .bind(&symbols)
            .bind(&pe)
            .bind(&pb)
            .execute(database::get_connection())
            .await?;
        Ok(result.rows_affected())
    }

    async fn sync_latest_quotes(&self) -> Result<u64> {
        Ok(sqlx::query(SYNC_LATEST_SQL)
            .execute(database::get_connection())
            .await?
            .rows_affected())
    }
}

#[cfg(test)]
mod tests;
