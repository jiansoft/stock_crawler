use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, NaiveDate};
use sqlx::{Postgres, postgres::PgQueryResult, Transaction};

use crate::internal::database;

#[derive(sqlx::FromRow, Default, Debug)]
pub struct DailyMoneyHistoryDetail {
    pub date: NaiveDate,
    pub created_time: DateTime<Local>,
    pub updated_time: DateTime<Local>,
    pub security_code: String,
    pub total_shares: i64,
    pub serial: i64,
    pub previous_day_market_value: f64,
    pub average_unit_price_per_share: f64,
    pub ratio: f64,
    pub previous_day_profit_and_loss: f64,
    pub market_value: f64,
    pub cost: f64,
    pub transfer_tax: f64,
    pub profit_and_loss: f64,
    pub profit_and_loss_percentage: f64,
    pub previous_day_profit_and_loss_percentage: f64,
    pub closing_price: f64,
    pub member_id: i32,
}

impl DailyMoneyHistoryDetail {
    pub async fn delete(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = "DELETE FROM daily_money_history_detail WHERE date = $1;";
        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to delete({}) daily_money_history_detail from database",
            &date
        ))
    }

    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let one_month_ago = date - Duration::days(30);
        let sql = format!(
            r#"
WITH total_ownership_details AS (
    SELECT
        security_code,
        member_id,
        share_quantity,
        holding_cost,
        share_price_average
    FROM
        stock_ownership_details
    WHERE
        is_sold = FALSE
        AND created_time <= '{0} 15:00:00'
),
ownership_details AS (
    SELECT
        security_code,
        member_id,
        sum(share_quantity) AS total_share,
        sum(holding_cost) AS average_cost,
        avg(share_price_average) AS average_unit_price
    FROM
        total_ownership_details
    GROUP BY
        security_code,
        member_id
    UNION
    SELECT
        security_code,
        0 AS member_id,
        sum(share_quantity) AS total_share,
        sum(holding_cost) AS average_cost,
        avg(share_price_average) AS average_unit_price
    FROM
        total_ownership_details
    GROUP BY
        security_code
),
daily_quotes AS (
    SELECT
        "Serial",
        "SecurityCode",
        "Date" AS date,
        "ClosingPrice"
    FROM
        "DailyQuotes"
    WHERE
        "Date" >= '{1}'
        AND "Date" <= '{0}'
        AND "SecurityCode" IN (
            SELECT
                "SecurityCode"
            FROM
                total_ownership_details
        )
),
prev_daily_quotes AS (
    SELECT
        "SecurityCode",
        "ClosingPrice"
    FROM
        daily_quotes
    WHERE
        "Serial" IN (
            SELECT
                max("Serial") AS serial
            FROM
                daily_quotes
            WHERE
                date < '{0}'
            GROUP BY
                "SecurityCode"
        )
),
today_daily_quotes AS (
    SELECT
        "SecurityCode",
        "ClosingPrice"
    FROM
        daily_quotes
    WHERE
        "Serial" IN (
            SELECT
                max("Serial") AS serial
            FROM
                daily_quotes
            GROUP BY
                "SecurityCode"
        )
),
money_history_detail AS (
    SELECT
        od.member_id,
        TO_DATE('{0}', 'YYYY-MM-DD') AS date,
        od.security_code,
        c."Name" AS name,
        od.total_share,
        tdq."ClosingPrice" AS closing_price,
        od.total_share * tdq."ClosingPrice" AS market_value,
        od.total_share * pdq."ClosingPrice" AS previous_day_market_value,
        od.total_share * tdq."ClosingPrice" - od.total_share * pdq."ClosingPrice" AS previous_day_profit_and_loss,
        od.average_cost,
        od.total_share * tdq."ClosingPrice" + od.average_cost AS reference_profit_and_loss,
        od.total_share * tdq."ClosingPrice" * 0.003 AS transfer_tax,
        -od.average_cost / od.total_share AS average_price
    FROM
        ownership_details AS od
        INNER JOIN stocks AS c ON od.security_code = c.stock_symbol
        INNER JOIN today_daily_quotes AS tdq ON od.security_code = tdq."SecurityCode"
        INNER JOIN prev_daily_quotes AS pdq ON od.security_code = pdq."SecurityCode"
),
market_value AS (
    SELECT
        member_id,
        sum(market_value) AS member_market_value_sum
    FROM
        money_history_detail
    GROUP BY
        member_id
)
INSERT INTO daily_money_history_detail(
    member_id,
    date,
    security_code,
    closing_price,
    total_shares,
    cost,
    average_unit_price_per_share,
    market_value,
    ratio,
    transfer_tax,
    profit_and_loss,
    profit_and_loss_percentage,
    created_time,
    updated_time,
    previous_day_market_value,
    previous_day_profit_and_loss,
    previous_day_profit_and_loss_percentage
)
SELECT
    mhd.member_id,
    date,
    security_code,
    closing_price,
    total_share,
    average_cost,
    ROUND(average_price, 4) AS average_price,
    market_value,
    ROUND(market_value / member_market_value_sum * 100, 4) AS ratio,
    transfer_tax,
    reference_profit_and_loss,
    CASE WHEN average_cost <> 0 THEN
        ROUND((market_value - abs(average_cost)) / abs(average_cost) * 100, 4)
    ELSE
        100
    END AS profit_and_loss_percentage,
    now(),
    now(),
    previous_day_market_value,
    previous_day_profit_and_loss,
    ROUND(((market_value - abs(previous_day_market_value)) / abs(previous_day_market_value)) * 100, 4) AS previous_day_profit_and_loss_percentage
FROM
    money_history_detail AS mhd
    INNER JOIN market_value AS mv ON mhd.member_id = mv.member_id
ON CONFLICT (date, security_code, member_id) DO UPDATE SET
    closing_price = EXCLUDED.closing_price,
    total_shares = EXCLUDED.total_shares,
    cost = EXCLUDED.cost,
    average_unit_price_per_share = EXCLUDED.average_unit_price_per_share,
    market_value = EXCLUDED.market_value,
    ratio = EXCLUDED.ratio,
    transfer_tax = EXCLUDED.transfer_tax,
    profit_and_loss = EXCLUDED.profit_and_loss,
    profit_and_loss_percentage = EXCLUDED.profit_and_loss_percentage,
    previous_day_market_value = EXCLUDED.previous_day_market_value
            "#,
            date, one_month_ago
        );

        let query = sqlx::query(&sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to upsert({}) daily_money_history_detail from database",
            date
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
        logging::debug_file_async("開始 DailyMoneyHistoryDetail::delete_and_upsert".to_string());
        let current_date = NaiveDate::parse_from_str("2023-08-05", "%Y-%m-%d").unwrap();
        let mut tx = database::get_tx().await.ok();

        DailyMoneyHistoryDetail::delete(current_date, &mut tx)
            .await
            .expect("DailyMoneyHistoryDetail::delete is failed");

        match DailyMoneyHistoryDetail::upsert(current_date, &mut tx).await {
            Ok(r) => {
                logging::debug_file_async(format!("DailyMoneyHistoryDetail::upsert:{:#?}", r));
                tx.unwrap()
                    .commit()
                    .await
                    .expect("tx.unwrap().commit() is failed");
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to DailyMoneyHistoryDetail::delete_and_upsert because {:?}",
                    why
                ));
                tx.unwrap()
                    .rollback()
                    .await
                    .expect("tx.unwrap().rollback() is failed");
            }
        }

        logging::debug_file_async("結束 DailyMoneyHistoryDetail::delete_and_upsert".to_string());
    }
}
