//! 盤後資料筆數（[`AfterHoursDataQuery`]）的 PostgreSQL 實作。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::quote::after_hours::{AfterHoursCounts, AfterHoursDataQuery},
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// `$1` 每一天的加權指數、`chip_daily` 與 `qfii_history` 筆數；三張表都以日期為前導索引。
const AFTER_HOURS_COUNTS_SQL: &str = r#"
SELECT d.date,
       (SELECT COUNT(*) FROM "index" i WHERE i.category = 'TAIEX' AND i."date" = d.date) AS index,
       (SELECT COUNT(*) FROM chip_daily c WHERE c."date" = d.date) AS chip,
       (SELECT COUNT(*) FROM qfii_history q WHERE q.date = d.date) AS foreign_holding
FROM UNNEST($1::date[]) WITH ORDINALITY AS d(date, ord)
ORDER BY d.ord
"#;

#[async_trait]
impl AfterHoursDataQuery for PgQuoteRepository {
    async fn count_after_hours(&self, dates: &[NaiveDate]) -> Result<Vec<AfterHoursCounts>> {
        let rows: Vec<(NaiveDate, i64, i64, i64)> = sqlx::query_as(AFTER_HOURS_COUNTS_SQL)
            .bind(dates)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(date, index, chip, foreign_holding)| AfterHoursCounts {
                date,
                index,
                chip,
                foreign_holding,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每天的指數、籌碼與外資持股筆數依傳入的日期順序回傳，沒有資料的日子為 0。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn count_after_hours_per_date() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 count_after_hours_per_date：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let day = |d| NaiveDate::from_ymd_opt(1990, 8, d).expect("有效日期");
        let symbols = ["79979QA", "79979QB"];
        let cleanup = || async {
            for sql in [
                "DELETE FROM chip_daily WHERE stock_symbol = ANY($1)",
                "DELETE FROM qfii_history WHERE stock_symbol = ANY($1)",
            ] {
                sqlx::query(sql)
                    .bind(&symbols[..])
                    .execute(pool)
                    .await
                    .expect("清除測試資料");
            }
            sqlx::query(
                r#"DELETE FROM "index" WHERE category = 'TAIEX' AND "date" BETWEEN $1 AND $2"#,
            )
            .bind(day(1))
            .bind(day(3))
            .execute(pool)
            .await
            .expect("清除測試指數");
        };
        cleanup().await;
        sqlx::query(r#"INSERT INTO "index" (category, "date", "index") VALUES ('TAIEX', $1, 100)"#)
            .bind(day(1))
            .execute(pool)
            .await
            .expect("種入指數");
        sqlx::query(
            r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net)
               VALUES ($1, '79979QA', 1, 0, 0), ($1, '79979QB', 1, 0, 0), ($2, '79979QA', 1, 0, 0)"#,
        )
        .bind(day(1))
        .bind(day(2))
        .execute(pool)
        .await
        .expect("種入籌碼");
        sqlx::query("INSERT INTO qfii_history (stock_symbol, date) VALUES ('79979QB', $1)")
            .bind(day(2))
            .execute(pool)
            .await
            .expect("種入外資持股");

        let counts = PgQuoteRepository::new()
            .count_after_hours(&[day(3), day(2), day(1)])
            .await
            .expect("查詢筆數");
        cleanup().await;
        assert_eq!(
            counts,
            [
                AfterHoursCounts {
                    date: day(3),
                    ..Default::default()
                },
                AfterHoursCounts {
                    date: day(2),
                    index: 0,
                    chip: 1,
                    foreign_holding: 1,
                },
                AfterHoursCounts {
                    date: day(1),
                    index: 1,
                    chip: 2,
                    foreign_holding: 0,
                },
            ]
        );
    }
}
