use anyhow::*;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::postgres::PgQueryResult;

use crate::infra::database;

/// 個股歷史價格與股價淨值比極值紀錄。
#[derive(sqlx::Type, sqlx::FromRow, Debug, Default, Clone)]
pub struct QuoteHistoryRecord {
    /// 歷史最高價發生日期。
    pub maximum_price_date_on: NaiveDate,
    /// 歷史最低價發生日期。
    pub minimum_price_date_on: NaiveDate,
    /// 歷史最高股價淨值比發生日期。
    pub maximum_price_to_book_ratio_date_on: NaiveDate,
    /// 歷史最低股價淨值比發生日期。
    pub minimum_price_to_book_ratio_date_on: NaiveDate,
    /// 股票代號。
    pub security_code: String,
    /// 歷史最高價。
    pub maximum_price: Decimal,
    /// 歷史最低價。
    pub minimum_price: Decimal,
    /// 歷史最高股價淨值比。
    pub maximum_price_to_book_ratio: Decimal,
    /// 歷史最低股價淨值比。
    pub minimum_price_to_book_ratio: Decimal,
}

impl QuoteHistoryRecord {
    /// 建立指定股票代號的歷史紀錄預設值。
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

    /// 依日報價重建指定股票的歷史最高、最低價與日期，回傳實際新增或變更的筆數。
    ///
    /// 每日收盤只拿當天報價和舊紀錄比較，回補過去的行情不會反映到這裡；回補後用這個
    /// 從整段日K重算。最低價只看大於 0 的列（無成交）；同價取最早一天，與逐日累積一致。
    /// 股價淨值比的極值需要當時的每股淨值，無法回推，保持原值。
    pub async fn rebuild_price_extremes(stock_symbols: &[String]) -> Result<u64> {
        if stock_symbols.is_empty() {
            return Ok(0);
        }
        let sql = r#"
WITH extremes AS (
    SELECT stock_symbol,
        max("HighestPrice") AS max_price,
        min("LowestPrice") FILTER (WHERE "LowestPrice" > 0) AS min_price
    FROM "DailyQuotes"
    WHERE stock_symbol = ANY($1)
    GROUP BY stock_symbol
    HAVING max("HighestPrice") > 0
), dated AS (
    SELECT e.stock_symbol,
        round(e.max_price, 4) AS max_price,
        (SELECT min(q."Date") FROM "DailyQuotes" AS q
          WHERE q.stock_symbol = e.stock_symbol AND q."HighestPrice" = e.max_price) AS max_on,
        round(COALESCE(e.min_price, 0), 4) AS min_price,
        COALESCE((SELECT min(q."Date") FROM "DailyQuotes" AS q
          WHERE q.stock_symbol = e.stock_symbol AND q."LowestPrice" = e.min_price), DATE '1970-01-01') AS min_on
    FROM extremes AS e
)
INSERT INTO quote_history_record (security_code, maximum_price, maximum_price_date_on, minimum_price, minimum_price_date_on)
SELECT stock_symbol, max_price, max_on, min_price, min_on FROM dated
ON CONFLICT (security_code) DO UPDATE SET
    maximum_price = EXCLUDED.maximum_price,
    maximum_price_date_on = EXCLUDED.maximum_price_date_on,
    minimum_price = EXCLUDED.minimum_price,
    minimum_price_date_on = EXCLUDED.minimum_price_date_on
WHERE (quote_history_record.maximum_price, quote_history_record.maximum_price_date_on,
       quote_history_record.minimum_price, quote_history_record.minimum_price_date_on)
      IS DISTINCT FROM
      (EXCLUDED.maximum_price, EXCLUDED.maximum_price_date_on,
       EXCLUDED.minimum_price, EXCLUDED.minimum_price_date_on)
"#;
        let result = sqlx::query(sql)
            .bind(stock_symbols)
            .execute(database::get_connection())
            .await
            .with_context(|| {
                format!(
                    "Failed to rebuild price extremes of {} symbols",
                    stock_symbols.len()
                )
            })?;
        Ok(result.rows_affected())
    }

    /// 新增或更新單一股票的歷史極值資料。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
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
            .bind(&self.security_code)
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

    use super::*;

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert() {
        dotenvy::dotenv().ok();
        tracing::info!("開始 upsert");
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
            Ok(_) => tracing::info!("{:#?}", qhr),
            Err(why) => {
                tracing::error!("Failed to upsert because {:?}", why);
            }
        }

        // 測試結束後移除假代號資料列，避免測試資料殘留在資料庫
        // （曾在正式庫發現本測試留下的 '79979' 紀錄）。
        if let Err(why) = sqlx::query("DELETE FROM quote_history_record WHERE security_code = $1")
            .bind(&qhr.security_code)
            .execute(database::get_connection())
            .await
        {
            tracing::error!("Failed to cleanup test row because {:?}", why);
        }

        tracing::info!("結束 upsert");
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch() {
        dotenvy::dotenv().ok();
        tracing::info!("開始 fetch");

        match QuoteHistoryRecord::fetch().await {
            Ok(qhr) => tracing::info!("{:#?}", qhr),
            Err(why) => {
                tracing::error!("Failed to fetch because {:?}", why);
            }
        }

        tracing::info!("結束 fetch");
    }

    /// 依日K重建最高、最低價：最低價略過 0（無成交），同價取最早一天；淨值比極值不動；
    /// 重跑時沒有變化不寫入。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn rebuild_price_extremes_from_daily_quotes() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 rebuild_price_extremes_from_daily_quotes：無資料庫連接");
            return;
        }
        let symbol = "79979QH".to_string();
        let cleanup = || async {
            for sql in [
                r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
                "DELETE FROM quote_history_record WHERE security_code = $1",
            ] {
                sqlx::query(sql)
                    .bind("79979QH")
                    .execute(database::get_connection())
                    .await
                    .expect("清除測試資料");
            }
        };
        cleanup().await;

        // 01-02 與 01-05 同為最高 12；01-03 無成交（最低 0）；01-04 最低 9。
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" (stock_symbol, "Date", "HighestPrice", "LowestPrice", "ClosingPrice")
               VALUES ($1, '2001-01-02', 12, 10, 11), ($1, '2001-01-03', 0, 0, 0),
                      ($1, '2001-01-04', 11, 9, 10), ($1, '2001-01-05', 12, 10, 11)"#,
        )
        .bind(&symbol)
        .execute(database::get_connection())
        .await
        .expect("寫入測試日K");
        let mut old = QuoteHistoryRecord::new(symbol.clone());
        old.maximum_price = dec!(5);
        old.maximum_price_to_book_ratio = dec!(3.3);
        old.upsert().await.expect("寫入舊紀錄");

        let symbols = [symbol.clone()];
        assert_eq!(
            QuoteHistoryRecord::rebuild_price_extremes(&symbols)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            QuoteHistoryRecord::rebuild_price_extremes(&symbols)
                .await
                .unwrap(),
            0
        );

        let row: (Decimal, NaiveDate, Decimal, NaiveDate, Decimal) = sqlx::query_as(
            r#"SELECT maximum_price, maximum_price_date_on, minimum_price, minimum_price_date_on,
                      "maximum_price-to-book_ratio"
               FROM quote_history_record WHERE security_code = $1"#,
        )
        .bind(&symbol)
        .fetch_one(database::get_connection())
        .await
        .expect("讀回紀錄");
        cleanup().await;

        let day = |d| NaiveDate::from_ymd_opt(2001, 1, d).unwrap();
        assert_eq!(row, (dec!(12), day(2), dec!(9), day(4), dec!(3.3)));
    }
}
