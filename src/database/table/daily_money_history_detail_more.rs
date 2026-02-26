use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::database;

/// 每日市值明細（交易批次層級）資料列。
///
/// 與 [`crate::database::table::daily_money_history_detail::DailyMoneyHistoryDetail`] 不同，
/// 這張表保留每筆持股來源（交易日期）維度，方便追蹤批次成本與損益。
#[derive(Debug, sqlx::FromRow)]
pub struct DailyMoneyHistoryDetailMore {
    /// 主鍵序號。
    pub serial: i64,
    /// 會員識別碼（0 代表全體聚合）。
    pub member_id: i64,
    /// 統計日期。
    pub date: NaiveDate,
    /// 原始買入/建立交易日期。
    pub transaction_date: NaiveDate,
    /// 股票代號。
    pub security_code: String,
    /// 當日收盤價。
    pub closing_price: Decimal,
    /// 此批次持有股數。
    pub number_of_shares_held: i64,
    /// 此批次每股成本。
    pub unit_price_per_share: Decimal,
    /// 此批次總成本。
    pub cost: Decimal,
    /// 此批次當日市值。
    pub market_value: Decimal,
    /// 此批次當日損益金額。
    pub profit_and_loss: Decimal,
    /// 此批次當日損益百分比。
    pub profit_and_loss_percentage: Decimal,
    /// 建立時間。
    pub created_time: chrono::DateTime<chrono::Local>,
    /// 最後更新時間。
    pub updated_time: chrono::DateTime<chrono::Local>,
}

impl DailyMoneyHistoryDetailMore {
    /// 刪除指定日期的 `daily_money_history_detail_more` 全部資料。
    ///
    /// 此方法通常作為重建流程的前置步驟，先清除同日舊資料，
    /// 再由 [`Self::upsert`] 重新寫入最新結果。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤；若呼叫端有提供 transaction，
    /// 是否回滾由呼叫端控制。
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

    /// 重建指定日期的 `daily_money_history_detail_more` 明細資料。
    ///
    /// 這個方法會以未賣出的 `stock_ownership_details` 為基礎，
    /// 搭配當日 `daily_money_history_detail` 的收盤價計算每筆交易批次的
    /// 市值、損益與損益百分比，並同時產生 member 與全局 (`member_id = 0`) 的資料列。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤；若呼叫端有提供 transaction，
    /// 是否回滾由呼叫端控制。
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
