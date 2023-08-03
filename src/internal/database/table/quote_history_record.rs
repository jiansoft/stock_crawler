use anyhow::*;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::postgres::PgQueryResult;

use crate::internal::database;

#[derive(sqlx::Type, sqlx::FromRow, Debug, Default)]
pub struct QuoteHistoryRecord {
    // 歷史最高價出現在哪一天
    pub maximum_price_date_on: NaiveDate,
    // 歷史最低價出現在哪一天
    pub minimum_price_date_on: NaiveDate,
    // 歷史最高股價淨值比出現在哪一天
    pub maximum_price_to_book_ratio_date_on: NaiveDate,
    // 歷史最低股價淨值比出現在哪一天
    pub minimum_price_to_book_ratio_date_on: NaiveDate,
    // 股票代號
    pub security_code: String,
    // 歷史最高價
    pub maximum_price: Decimal,
    // 歷史最低價
    pub minimum_price: Decimal,
    // 歷史最高股價淨值比
    pub maximum_price_to_book_ratio: Decimal,
    // 歷史最低股價淨值比
    pub minimum_price_to_book_ratio: Decimal,
}

impl QuoteHistoryRecord {
    pub fn new(security_code: String) -> Self {
        QuoteHistoryRecord {
            security_code,
            ..Default::default()
        }
    }

    /// 取得所有股票歷史最高、最低等數據
    pub async fn fetch() -> Result<Vec<QuoteHistoryRecord>> {
        sqlx::query_as::<_, QuoteHistoryRecord>(
            r#"
SELECT
    security_code,
    maximum_price,
    maximum_price_date_on,
    minimum_price,
    minimum_price_date_on,
    "maximum_price-to-book_ratio" as maximum_price_to_book_ratio,
    "maximum_price-to-book_ratio_date_on" as maximum_price_to_book_ratio_date_on,
    "minimum_price-to-book_ratio" as minimum_price_to_book_ratio,
    "minimum_price-to-book_ratio_date_on" as minimum_price_to_book_ratio_date_on
FROM
    quote_history_record
"#,
        )
        .fetch_all(database::get_connection())
        .await
        .context("Failed to QuoteHistoryRecord.fetch from database")
    }

    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO
    quote_history_record (
        security_code,
        maximum_price,
        maximum_price_date_on,
        minimum_price,
        minimum_price_date_on,
        "maximum_price-to-book_ratio",
        "maximum_price-to-book_ratio_date_on",
        "minimum_price-to-book_ratio",
        "minimum_price-to-book_ratio_date_on"
    )
VALUES
    (
      $1, $2, $3, $4, $5, $6, $7, $8, $9
    )
ON CONFLICT
    (security_code)
DO UPDATE
SET
    maximum_price = EXCLUDED.maximum_price,
    maximum_price_date_on = EXCLUDED.maximum_price_date_on,
    minimum_price = EXCLUDED.minimum_price,
    minimum_price_date_on = EXCLUDED.minimum_price_date_on,
    "maximum_price-to-book_ratio" = EXCLUDED."maximum_price-to-book_ratio",
    "maximum_price-to-book_ratio_date_on" = EXCLUDED. "maximum_price-to-book_ratio_date_on",
    "minimum_price-to-book_ratio" = EXCLUDED."minimum_price-to-book_ratio",
    "minimum_price-to-book_ratio_date_on" = EXCLUDED."minimum_price-to-book_ratio_date_on"
"#;
        sqlx::query(sql)
            .bind(self.security_code.as_str())
            .bind(self.maximum_price)
            .bind(self.maximum_price_date_on)
            .bind(self.minimum_price)
            .bind(self.minimum_price_date_on)
            .bind(self.maximum_price_to_book_ratio)
            .bind(self.maximum_price_to_book_ratio_date_on)
            .bind(self.minimum_price_to_book_ratio)
            .bind(self.minimum_price_to_book_ratio_date_on)
            .execute(database::get_connection())
            .await
            .context(format!("Failed to upsert({:#?}) from database", self))
    }
}

#[cfg(test)]
mod tests {
    use core::result::Result::Ok;

    use rust_decimal_macros::dec;

    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 upsert".to_string());
        let date = NaiveDate::from_ymd_opt(2023, 8, 2);
        let mut qhr = QuoteHistoryRecord::new("79979".to_string());
        qhr.maximum_price = dec!(1.1);
        qhr.maximum_price_date_on = date.unwrap();
        qhr.maximum_price_to_book_ratio = dec!(1.11);
        qhr.maximum_price_to_book_ratio_date_on = date.unwrap();
        qhr.minimum_price = dec!(2);
        qhr.minimum_price_date_on = date.unwrap();
        qhr.minimum_price_to_book_ratio = dec!(2.2);
        qhr.minimum_price_to_book_ratio_date_on = date.unwrap();

        match qhr.upsert().await {
            Ok(_) => logging::info_file_async(format!("{:#?}", qhr)),
            Err(why) => {
                logging::error_file_async(format!("Failed to upsert because {:?}", why));
            }
        }

        logging::info_file_async("結束 upsert".to_string());
    }

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 fetch".to_string());

        match QuoteHistoryRecord::fetch().await {
            Ok(qhr) => logging::info_file_async(format!("{:#?}", qhr)),
            Err(why) => {
                logging::error_file_async(format!("Failed to fetch because {:?}", why));
            }
        }

        logging::info_file_async("結束 fetch".to_string());
    }
}
