//! 盤後資料筆數（[`AfterHoursDataQuery`]）的 PostgreSQL 實作。

use anyhow::Result;
use async_trait::async_trait;
use jiff::civil::Date;
use jiff_sqlx::ToSqlx;

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
    async fn count_after_hours(&self, dates: &[Date]) -> Result<Vec<AfterHoursCounts>> {
        let rows: Vec<(Date, i64, i64, i64)> =
            sqlx::query_as::<_, (jiff_sqlx::Date, i64, i64, i64)>(AFTER_HOURS_COUNTS_SQL)
                .bind(
                    dates
                        .iter()
                        .map(|value| value.to_sqlx())
                        .collect::<Vec<_>>(),
                )
                .fetch_all(database::get_connection())
                .await
                .map(|rows| {
                    rows.into_iter()
                        .map(|(c0, c1, c2, c3)| {
                            (
                                crate::infra::database::jiff_types::from_wire(c0),
                                c1,
                                c2,
                                c3,
                            )
                        })
                        .collect::<Vec<_>>()
                })?;
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
mod tests;
