use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::database;

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
        let sql = r#"
INSERT INTO daily_money_history (date, sum, eddie, unice)
WITH base_calc AS (
    -- 僅執行一次核心 Join，大幅減少 I/O 與 CPU 開銷
    SELECT 
        od.member_id,
        (od.share_quantity * dq."ClosingPrice") AS market_value
    FROM stock_ownership_details od
    INNER JOIN "DailyQuotes" dq ON od.security_code = dq."stock_symbol"
    WHERE od.is_sold = FALSE 
      AND od.date <= $1
      AND dq."Date" = $1
)
SELECT 
    $1 AS date,
    COALESCE(SUM(market_value), 0) AS sum,
    -- 使用 PostgreSQL FILTER 語法進行條件式聚合
    COALESCE(SUM(market_value) FILTER (WHERE member_id = 1), 0) AS eddie,
    COALESCE(SUM(market_value) FILTER (WHERE member_id != 1), 0) AS unice
FROM base_calc
ON CONFLICT (date) DO UPDATE SET
    sum = EXCLUDED.sum,
    eddie = EXCLUDED.eddie,
    unice = EXCLUDED.unice,
    updated_time = NOW();
"#;

        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.map_err(|why| {
            anyhow!(
                "Failed to daily_money_history::upsert({}) from database because {:?}",
                date,
                why
            )
        })
    }

}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
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
