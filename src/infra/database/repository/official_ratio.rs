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
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 只填 0：不覆寫既有值、官方沒有值不動；缺值日期只看指定市場、排除 ETF 與無成交。
    /// 以官方為準：處理所有交易日、不同就覆寫、官方沒有值寫 0。
    /// 用假代號與 2001 年的日期，結束時清除。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn fill_official_ratios_by_policy() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 fill_official_ratios_by_policy：無資料庫連接");
            return;
        }
        let cleanup = || async {
            for sql in [
                r#"DELETE FROM "DailyQuotes" WHERE stock_symbol IN ('79979A', '79979B', '0079979')"#,
                "DELETE FROM last_daily_quotes WHERE stock_symbol IN ('79979A', '79979B', '0079979')",
                "DELETE FROM stocks WHERE stock_symbol IN ('79979A', '79979B', '0079979')",
            ] {
                let _ = sqlx::query(sql).execute(pool).await;
            }
        };
        cleanup().await;
        for sql in [
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ('79979A', '上市測試', '79979A', 2, 1), ('79979B', '上櫃測試', '79979B', 4, 1), ('0079979', 'ETF 測試', '0079979', 2, 1)"#,
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "ClosingPrice", "PriceEarningRatio", "price-to-book_ratio")
               VALUES ('2001-03-01', '79979A', 2001, 3, 1, 50, 0, 0),
                      ('2001-03-02', '79979A', 2001, 3, 2, 50, 12, 0),
                      ('2001-03-05', '79979A', 2001, 3, 5, 0, 0, 0),
                      ('2001-03-02', '79979B', 2001, 3, 2, 30, 0, 1.5),
                      ('2001-03-05', '79979B', 2001, 3, 5, 31, 11.8, 1.5),
                      ('2001-03-06', '79979B', 2001, 3, 6, 32, 7, 1.5),
                      ('2001-03-06', '0079979', 2001, 3, 6, 20, 0, 0)"#,
        ] {
            sqlx::query(sql).execute(pool).await.expect("種入測試資料");
        }
        let repo = PgQuoteRepository::new();
        let from = NaiveDate::from_ymd_opt(2001, 3, 1).expect("有效日期");
        let to = NaiveDate::from_ymd_opt(2001, 3, 31).expect("有效日期");

        // 上市：03-01、03-02 有缺；03-05 沒成交、ETF 不算。
        let listed = repo
            .fetch_dates_to_update(2, from, to, PePolicy::FillZeroOnly)
            .await
            .expect("查詢");
        assert_eq!(
            listed,
            vec![from, NaiveDate::from_ymd_opt(2001, 3, 2).expect("有效日期")]
        );

        let day = NaiveDate::from_ymd_opt(2001, 3, 2).expect("有效日期");
        let ratios = [
            OfficialRatio {
                stock_symbol: "79979A".to_owned(),
                price_earning_ratio: Some(dec!(15)),
                price_to_book_ratio: Some(dec!(2.5)),
            },
            OfficialRatio {
                stock_symbol: "79979B".to_owned(),
                price_earning_ratio: None,
                price_to_book_ratio: Some(dec!(9.9)),
            },
        ];
        // 79979A 只補淨值比（本益比 12 不覆寫）；79979B 本益比官方沒值、淨值比已有值 → 不動。
        assert_eq!(
            repo.fill_official_ratios(day, &ratios, PePolicy::FillZeroOnly)
                .await
                .expect("更新"),
            1
        );
        let (pe, pb): (Decimal, Decimal) = sqlx::query_as(
            r#"SELECT "PriceEarningRatio", "price-to-book_ratio" FROM "DailyQuotes" WHERE stock_symbol = '79979A' AND "Date" = $1"#,
        )
        .bind(day)
        .fetch_one(pool)
        .await
        .expect("查詢");
        assert_eq!((pe, pb), (dec!(12), dec!(2.5)));
        let otc_pb: Decimal = sqlx::query_scalar(
            r#"SELECT "price-to-book_ratio" FROM "DailyQuotes" WHERE stock_symbol = '79979B'"#,
        )
        .fetch_one(pool)
        .await
        .expect("查詢");
        assert_eq!(otc_pb, dec!(1.5));
        assert_eq!(
            repo.fill_official_ratios(day, &[], PePolicy::FillZeroOnly)
                .await
                .expect("空清單"),
            0
        );

        // 上櫃以官方為準：處理所有交易日（不只缺值日）；與官方不同就覆寫、官方沒有值寫 0、
        // 已相同不碰；淨值比仍只填 0。
        let otc_dates = repo
            .fetch_dates_to_update(4, from, to, PePolicy::Authoritative)
            .await
            .expect("查詢");
        assert_eq!(otc_dates.len(), 3);
        let otc = |pe| {
            [OfficialRatio {
                stock_symbol: "79979B".to_owned(),
                price_earning_ratio: pe,
                price_to_book_ratio: Some(dec!(9.9)),
            }]
        };
        let date = |d| NaiveDate::from_ymd_opt(2001, 3, d).expect("有效日期");
        let fill = |d, pe| {
            let repo = &repo;
            async move {
                repo.fill_official_ratios(date(d), &otc(pe), PePolicy::Authoritative)
                    .await
                    .expect("更新")
            }
        };
        assert_eq!(fill(5, Some(dec!(18.55))).await, 1);
        assert_eq!(fill(5, Some(dec!(18.55))).await, 0, "已相同不再更新");
        assert_eq!(fill(6, None).await, 1);
        assert_eq!(fill(2, None).await, 0, "原本就是 0、淨值比已有值");
        let otc_rows: Vec<(Decimal, Decimal)> = sqlx::query_as(
            r#"SELECT "PriceEarningRatio", "price-to-book_ratio" FROM "DailyQuotes" WHERE stock_symbol = '79979B' ORDER BY "Date""#,
        )
        .fetch_all(pool)
        .await
        .expect("查詢");
        assert_eq!(
            otc_rows,
            vec![
                (dec!(0), dec!(1.5)),
                (dec!(18.55), dec!(1.5)),
                (dec!(0), dec!(1.5))
            ]
        );

        // 最新報價表跟著同日日線更新；已相同的不再更新。
        sqlx::query(
            "INSERT INTO last_daily_quotes (date, stock_symbol) VALUES ('2001-03-05', '79979B')",
        )
        .execute(pool)
        .await
        .expect("種入最新報價");
        assert!(repo.sync_latest_quotes().await.expect("同步") >= 1);
        let latest: (Decimal, Decimal) = sqlx::query_as(
            r#"SELECT price_earning_ratio, "price-to-book_ratio" FROM last_daily_quotes WHERE stock_symbol = '79979B'"#,
        )
        .fetch_one(pool)
        .await
        .expect("查詢");
        assert_eq!(latest, (dec!(18.55), dec!(1.5)));

        cleanup().await;
    }
}
