use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{Postgres, postgres::PgQueryResult, Transaction};

use crate::internal::database;

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
        let sql = format!(
            r#"
with stock AS (
    SELECT
        dmhd.member_id,
        sod.created_time AS create_time,
        sod.security_code,
        s."Name" AS name,
        sod.share_quantity AS number_of_shares_held,
        sod.holding_cost AS average_cost,
        sod.share_price_average AS amount_per_share,
        dmhd.closing_price,
        dmhd.closing_price * sod.share_quantity AS market_value,
        dmhd.closing_price * sod.share_quantity + sod.holding_cost AS profit_and_loss,
        CASE WHEN sod.holding_cost != 0 THEN
            round((dmhd.closing_price * sod.share_quantity + sod.holding_cost) / abs(sod.holding_cost) * 100, 4)
        ELSE
            100
        END AS profit_and_loss_percentage,
        dmhd.date
    FROM
        stocks AS s
        INNER JOIN stock_ownership_details AS sod ON sod.security_code = s.stock_symbol
        INNER JOIN daily_money_history_detail dmhd ON dmhd.security_code = sod.security_code
            AND dmhd.date = '{0}'
            AND sod.member_id = dmhd.member_id
    WHERE sod.is_sold = FALSE
    UNION ALL
    SELECT
        0 AS member_id,
        sod.created_time AS create_time,
        sod.security_code,
        s."Name" AS name,
        sod.share_quantity AS number_of_shares_held,
        sod.holding_cost AS average_cost,
        sod.share_price_average AS amount_per_share,
        dmhd.closing_price,
        dmhd.closing_price * sod.share_quantity AS market_value,
        dmhd.closing_price * sod.share_quantity + sod.holding_cost AS profit_and_loss,
        CASE WHEN sod.holding_cost != 0 THEN
            round((dmhd.closing_price * sod.share_quantity + sod.holding_cost) / abs(sod.holding_cost) * 100, 4)
        ELSE
            100
        END AS profit_and_loss_percentage,
        dmhd.date
    FROM
        stocks AS s
        INNER JOIN stock_ownership_details AS sod ON sod.security_code = s.stock_symbol
        INNER JOIN daily_money_history_detail dmhd ON dmhd.security_code = sod.security_code
            AND dmhd.date = '{0}'
            AND sod.member_id = dmhd.member_id
    WHERE sod.is_sold = FALSE)
INSERT INTO daily_money_history_detail_more (member_id, "date", transaction_date, security_code, closing_price, number_of_shares_held, unit_price_per_share,
    cost, market_value, profit_and_loss, profit_and_loss_percentage)
SELECT
    member_id,
	date,
    create_time,
    security_code,
    closing_price,
    number_of_shares_held,
    amount_per_share,
    average_cost,
    market_value,
    profit_and_loss,
    profit_and_loss_percentage
FROM
    stock
ORDER BY
    security_code,
    member_id,
    create_time;
"#,
            date
        );

        let query = sqlx::query(&sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to upsert({}) daily_money_history_detail_more from database",
            &date
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
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
                logging::debug_file_async(format!("Failed to DailyMoneyHistoryDetailMore::delete_and_upsert because {:?}", why));
                tx.unwrap()
                    .rollback()
                    .await
                    .expect("tx.unwrap().rollback() is failed");
            }
        }

        logging::debug_file_async("結束 DailyMoneyHistoryDetailMore::delete_and_upsert".to_string());
    }
}
