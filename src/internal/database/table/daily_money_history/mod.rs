use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::internal::database;

pub(crate) mod extension;

/// 每日市值變化歷史記錄
#[derive(sqlx::FromRow, Debug)]
pub struct DailyMoneyHistory {
    pub date: NaiveDate,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub unice: Decimal,
    pub eddie: Decimal,
    pub sum: Decimal,
}

impl DailyMoneyHistory {
    /* pub async fn fetch(date :NaiveDate) -> Result<> {
            let sql = format!("");
            sqlx::query_as::<_, DailyMoneyHistory>(&sql)
                .bind(date.year())
                .bind(date.format("%Y-%m-%d").to_string())
                .fetch_all(database::get_connection())
                .await
                .context(format!(
                    "Failed to fetch_stocks_with_dividends_on_date({}) from database",
                    date
                ))
        }
    */
    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = format!(
            r#"
WITH ownership_details AS (
	SELECT security_code, share_quantity, member_id
	FROM stock_ownership_details
	WHERE is_sold = false and date <= $1
),
 daily_quotes AS (
	SELECT "SecurityCode", "ClosingPrice"
	FROM "DailyQuotes"
	WHERE "Date" = $1 AND "SecurityCode" in (select security_code FROM ownership_details)
),
total AS (
	SELECT '{0}' AS "date", SUM(od.share_quantity * dq."ClosingPrice") AS "sum"
	FROM ownership_details od
	INNER JOIN daily_quotes dq ON od.security_code = dq."SecurityCode"
),
eddie AS (
	SELECT '{0}' AS "date", SUM(od.share_quantity * dq."ClosingPrice") AS "sum"
	FROM ownership_details od
	INNER JOIN daily_quotes dq ON od.security_code = dq."SecurityCode"
	WHERE od.member_id = 1
)
INSERT INTO daily_money_history (date, sum, eddie, unice)
SELECT
	TO_DATE(total."date",'YYYY-MM-DD') AS "date",
	"total"."sum" AS sum,
	"eddie"."sum" AS eddie,
	"total"."sum" - "eddie"."sum" AS unice
FROM total
INNER JOIN eddie ON total."date" = eddie."date"
ON CONFLICT (date) DO UPDATE SET
	sum = EXCLUDED.sum,
	eddie = EXCLUDED.eddie,
	unice = EXCLUDED.unice,
	updated_time = now();
"#,
            date
        );

        let query = sqlx::query(&sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        match result {
            Ok(r) => Ok(r),
            Err(why) => Err(anyhow!(
                "Failed to daily_money_history::upsert({}) from database because {:?}",
                date,
                why
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 DailyMoneyHistory::upsert".to_string());
        let current_date = NaiveDate::parse_from_str("2023-08-30", "%Y-%m-%d").unwrap();
        let mut tx = database::get_tx().await.ok();
        match DailyMoneyHistory::upsert(current_date, &mut tx).await {
            Ok(r) => {
                logging::debug_file_async(format!("DailyMoneyHistory::upsert:{:#?}", r));
                tx.unwrap()
                    .commit()
                    .await
                    .expect("tx.unwrap().commit() is failed");
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to DailyMoneyHistory::upsert because {:?}",
                    why
                ));
                tx.unwrap()
                    .rollback()
                    .await
                    .expect("tx.unwrap().rollback() is failed");
            }
        }

        logging::debug_file_async("結束 DailyMoneyHistory::upsert".to_string());
    }
}
