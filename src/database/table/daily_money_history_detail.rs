use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, TimeDelta};
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::database;

/// 每日市值明細（持股層級）資料列。
///
/// 一筆資料代表特定 `date`、`member_id`、`security_code` 的聚合結果，
/// 包含持股數、成本、市值、損益與前一交易日比較欄位。
#[derive(sqlx::FromRow, Default, Debug)]
pub struct DailyMoneyHistoryDetail {
    /// 交易日期。
    pub date: NaiveDate,
    /// 建立時間。
    pub created_time: DateTime<Local>,
    /// 最後更新時間。
    pub updated_time: DateTime<Local>,
    /// 股票代號。
    pub security_code: String,
    /// 持有股數（同股票同 member 聚合後）。
    pub total_shares: i64,
    /// 主鍵序號。
    pub serial: i64,
    /// 前一交易日市值。
    pub previous_day_market_value: f64,
    /// 每股平均成本。
    pub average_unit_price_per_share: f64,
    /// 佔該 member 當日總市值比例（百分比）。
    pub ratio: f64,
    /// 與前一交易日相比的損益變化金額。
    pub previous_day_profit_and_loss: f64,
    /// 當日市值。
    pub market_value: f64,
    /// 累計成本（含符號約定）。
    pub cost: f64,
    /// 估算交易稅（市值 * 0.003）。
    pub transfer_tax: f64,
    /// 當日損益金額。
    pub profit_and_loss: f64,
    /// 當日損益百分比。
    pub profit_and_loss_percentage: f64,
    /// 相對前一交易日的損益百分比。
    pub previous_day_profit_and_loss_percentage: f64,
    /// 當日收盤價。
    pub closing_price: f64,
    /// 會員識別碼（0 代表全體聚合）。
    pub member_id: i32,
}

impl DailyMoneyHistoryDetail {
    /// 刪除指定日期的 `daily_money_history_detail` 全部資料。
    ///
    /// 主要用於重建流程的前置清理步驟，通常會與同日期的 `upsert` 搭配使用，
    /// 以避免舊資料殘留造成重複或不一致。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤；若呼叫端有提供 transaction，
    /// 錯誤會由呼叫端決定是否回滾。
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

    /// 重建指定日期的持股層級市值明細。
    ///
    /// 此流程會：
    /// 1. 聚合未賣出庫存（個人與全局）  
    /// 2. 取當日與前一日收盤價  
    /// 3. 計算市值、成本、占比、損益與前日對照欄位  
    /// 4. 以 `(date, security_code, member_id)` 做 upsert
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤；若呼叫端有提供 transaction，
    /// 是否回滾由呼叫端控制。
    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let one_month_ago = date - TimeDelta::try_days(30).unwrap();
        let sql = r#"
WITH ownership_data AS (
    -- 一次性計算個人與全局(member_id=0)的持有股數與成本，避免多次掃描
    SELECT 
        security_code,
        member_id,
        SUM(share_quantity) AS total_share,
        SUM(holding_cost) AS total_cost
    FROM stock_ownership_details
    WHERE is_sold = FALSE AND date <= $1
    GROUP BY GROUPING SETS ((security_code, member_id), (security_code))
),
normalized_ownership AS (
    SELECT 
        security_code,
        COALESCE(member_id, 0) as member_id,
        total_share,
        total_cost
    FROM ownership_data
),
quote_data AS (
    -- 使用視窗函數取得當日與前一日報價，rn=1 代表最新一筆
    SELECT 
        stock_symbol,
        "ClosingPrice" as today_price,
        LAG("ClosingPrice") OVER (PARTITION BY stock_symbol ORDER BY "Date") as yesterday_price,
        ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY "Date" DESC) as rn
    FROM "DailyQuotes"
    WHERE "Date" >= $2 AND "Date" <= $1
    AND "stock_symbol" IN (SELECT DISTINCT security_code FROM normalized_ownership)
),
latest_quotes AS (
    SELECT stock_symbol, today_price, COALESCE(yesterday_price, today_price) as yesterday_price
    FROM quote_data WHERE rn = 1
),
calc_base AS (
    SELECT
        od.member_id,
        $1 as date,
        od.security_code,
        od.total_share,
        lq.today_price as closing_price,
        od.total_share * lq.today_price as market_value,
        od.total_share * lq.yesterday_price as prev_market_value,
        od.total_cost as cost
    FROM normalized_ownership od
    JOIN latest_quotes lq ON od.security_code = lq.stock_symbol
),
member_totals AS (
    SELECT member_id, SUM(market_value) as total_mkt_val
    FROM calc_base GROUP BY member_id
)
INSERT INTO daily_money_history_detail (
    member_id, date, security_code, closing_price, total_shares, cost,
    average_unit_price_per_share, market_value, ratio, transfer_tax,
    profit_and_loss, profit_and_loss_percentage, created_time, updated_time,
    previous_day_market_value, previous_day_profit_and_loss, previous_day_profit_and_loss_percentage
)
SELECT
    cb.member_id, cb.date, cb.security_code, cb.closing_price, cb.total_share, cb.cost,
    ROUND(CAST(-cb.cost / cb.total_share AS numeric), 4),
    cb.market_value,
    ROUND(CAST(cb.market_value / NULLIF(mt.total_mkt_val, 0) * 100 AS numeric), 4),
    cb.market_value * 0.003,
    cb.market_value + cb.cost,
    CASE 
        WHEN cb.cost != 0 THEN ROUND(CAST((cb.market_value + cb.cost) / ABS(cb.cost) * 100 AS numeric), 4) 
        ELSE 100 
    END,
    NOW(), NOW(),
    cb.prev_market_value,
    cb.market_value - cb.prev_market_value,
    ROUND(CAST((cb.market_value - cb.prev_market_value) / NULLIF(cb.prev_market_value, 0) * 100 AS numeric), 4)
FROM calc_base cb
JOIN member_totals mt ON cb.member_id = mt.member_id
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
    previous_day_market_value = EXCLUDED.previous_day_market_value,
    updated_time = NOW();
"#;

        let query = sqlx::query(sql).bind(date).bind(one_month_ago);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to daily_money_history_detail::upsert({}) from database",
            date
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
