use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::database;

/// 每日市值垂直化總覽資料。
///
/// 一筆資料代表特定 `date` 與 `member_id` 的市值總額。
/// `member_id = 0` 保留為全體總和，其餘值對應實際會員。
#[derive(sqlx::FromRow, Debug)]
pub struct DailyMoneyHistoryMember {
    /// 交易日期。
    pub date: NaiveDate,
    /// 會員編號；0 代表全體總和。
    pub member_id: i64,
    /// 當日收盤市值總額。
    pub market_value: Decimal,
    /// 建立時間。
    pub created_at: DateTime<Local>,
    /// 最後更新時間。
    pub updated_at: DateTime<Local>,
}

impl DailyMoneyHistoryMember {
    /// 依指定日期重算並寫入每日市值垂直總覽。
    ///
    /// 設計目標：
    /// 1. 保持舊 `daily_money_history` 扁平表不變。
    /// 2. 新增可擴充的新表，避免新增會員時必須再加欄位。
    /// 3. 同一日同時寫入全體 (`member_id = 0`) 與所有已出現過的會員，
    ///    即使某會員當日無持股，也會保留 0 值資料列。
    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO daily_money_history_member (date, member_id, market_value)
WITH base_calc AS (
    SELECT
        od.member_id,
        (od.share_quantity * dq."ClosingPrice") AS market_value
    FROM stock_ownership_details od
    INNER JOIN "DailyQuotes" dq ON od.security_code = dq."stock_symbol"
    WHERE od.is_sold = FALSE
      AND od.date <= $1
      AND dq."Date" = $1
),
member_scope AS (
    -- 只要會員曾在當日前出現過，就為該日保留一筆 summary 列；
    -- 這樣未持倉的會員也能落成 0，而不需要靠 schema 固定欄位。
    SELECT DISTINCT od.member_id
    FROM stock_ownership_details od
    WHERE od.date <= $1
      AND od.member_id > 0
),
member_agg AS (
    SELECT
        bc.member_id,
        COALESCE(SUM(bc.market_value), 0) AS market_value
    FROM base_calc bc
    GROUP BY bc.member_id
),
final_rows AS (
    SELECT
        0::bigint AS member_id,
        COALESCE((SELECT SUM(bc.market_value) FROM base_calc bc), 0) AS market_value

    UNION ALL

    SELECT
        ms.member_id,
        COALESCE(ma.market_value, 0) AS market_value
    FROM member_scope ms
    LEFT JOIN member_agg ma ON ms.member_id = ma.member_id
)
SELECT
    $1 AS date,
    fr.member_id,
    fr.market_value
FROM final_rows fr
ON CONFLICT (date, member_id) DO UPDATE SET
    market_value = EXCLUDED.market_value,
    updated_time = NOW();
"#;

        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.map_err(|why| {
            anyhow!(
                "Failed to daily_money_history_member::upsert({}) from database because {:?}",
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
        logging::debug_file_async("開始 DailyMoneyHistoryMember::upsert".to_string());
        let current_date = NaiveDate::parse_from_str("2023-08-30", "%Y-%m-%d").unwrap();
        let mut tx = database::get_tx().await.ok();

        match DailyMoneyHistoryMember::upsert(current_date, &mut tx).await {
            Ok(r) => {
                logging::debug_file_async(format!("DailyMoneyHistoryMember::upsert:{:#?}", r));
                tx.unwrap()
                    .commit()
                    .await
                    .expect("tx.unwrap().commit() is failed");
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to DailyMoneyHistoryMember::upsert because {:?}",
                    why
                ));
                tx.unwrap()
                    .rollback()
                    .await
                    .expect("tx.unwrap().rollback() is failed");
            }
        }

        logging::debug_file_async("結束 DailyMoneyHistoryMember::upsert".to_string());
    }
}
