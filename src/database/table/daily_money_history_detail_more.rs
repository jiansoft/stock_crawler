use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::database;

#[derive(Debug, sqlx::FromRow)]
pub struct DailyMoneyHistoryDetailMore {
    pub serial: i64,
    pub member_id: i64,
    pub date: NaiveDate,
    pub transaction_date: NaiveDate,
    pub security_code: String,
    pub closing_price: Decimal,
    pub number_of_shares_held: i64,
    pub unit_price_per_share: Decimal,
    pub cost: Decimal,
    pub market_value: Decimal,
    pub profit_and_loss: Decimal,
    pub profit_and_loss_percentage: Decimal,
    pub created_time: chrono::DateTime<chrono::Local>,
    pub updated_time: chrono::DateTime<chrono::Local>,
}

impl DailyMoneyHistoryDetailMore {
    pub async fn delete(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = "DELETE FROM daily_money_history_detail_more WHERE date = $1;";
        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to delete({}) daily_money_history_detail_more from database",
            &date
        ))
    }

    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO daily_money_history_detail_more (
    member_id, "date", transaction_date, security_code, closing_price, 
    number_of_shares_held, unit_price_per_share, cost, market_value, 
    profit_and_loss, profit_and_loss_percentage
)
WITH raw_data AS (
    -- 一次性獲取基礎數據，移除對 stocks 表的多餘連結
    SELECT
        sod.member_id,
        sod.created_time::date as transaction_date,
        sod.security_code,
        sod.share_quantity,
        sod.holding_cost,
        sod.share_price_average,
        dmhd.closing_price,
        dmhd.date
    FROM stock_ownership_details sod
    JOIN daily_money_history_detail dmhd 
        ON sod.security_code = dmhd.security_code 
        AND sod.member_id = dmhd.member_id
    WHERE sod.is_sold = FALSE 
      AND dmhd.date = $1
),
aggregated_data AS (
    -- 透過 UNION ALL 快速映射個人與全局(member_id=0)數據
    SELECT * FROM raw_data
    UNION ALL
    SELECT 0 as member_id, transaction_date, security_code, share_quantity, holding_cost, 
           share_price_average, closing_price, date
    FROM raw_data
)
SELECT
    member_id,
    date,
    transaction_date,
    security_code,
    closing_price,
    share_quantity,
    share_price_average,
    holding_cost,
    (closing_price * share_quantity) as market_value,
    (closing_price * share_quantity + holding_cost) as profit_and_loss,
    CASE 
        WHEN holding_cost != 0 THEN 
            ROUND(CAST((closing_price * share_quantity + holding_cost) / ABS(holding_cost) * 100 AS numeric), 4)
        ELSE 100 
    END as profit_and_loss_percentage
FROM aggregated_data
ORDER BY security_code, member_id, transaction_date;
"#;

        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to daily_money_history_detail_more::upsert({}) from database",
            &date
        ))
    }

}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_delete_and_upsert() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 delete_and_upsert".to_string());

        let current_date = NaiveDate::parse_from_str("2023-08-05", "%Y-%m-%d").unwrap();
        let mut tx = database::get_tx().await.ok();

        DailyMoneyHistoryDetailMore::delete(current_date, &mut tx)
            .await
            .expect("DailyMoneyHistoryDetailMore::delete is failed");

        match DailyMoneyHistoryDetailMore::upsert(current_date, &mut tx).await {
            Ok(r) => {
                logging::debug_file_async(format!("DailyMoneyHistoryDetailMore::upsert:{:#?}", r));
                tx.unwrap()
                    .commit()
                    .await
                    .expect("tx.unwrap().commit() is failed");
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to DailyMoneyHistoryDetailMore::delete_and_upsert because {:?}",
                    why
                ));
                tx.unwrap()
                    .rollback()
                    .await
                    .expect("tx.unwrap().rollback() is failed");
            }
        }

        logging::debug_file_async(
            "結束 DailyMoneyHistoryDetailMore::delete_and_upsert".to_string(),
        );
    }
}
