use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use serde_derive::{Deserialize, Serialize};
use sqlx::{postgres::PgQueryResult, FromRow, Postgres, Transaction, Type};

use crate::database;

#[derive(Debug, Serialize, Deserialize, Type, FromRow)]
pub struct DailyStockPriceStats {
    pub date: NaiveDate,                   // 統計日期
    pub stock_exchange_market_id: i32,     // 市場類型 (TWSE: 2, TPEx: 4, ALL: 0)
    pub undervalued: i32,                  // 股價 <= 便宜價的股票數量
    pub fair_valued: i32,                  // 便宜價 < 股價 <= 合理價的股票數量
    pub overvalued: i32,                   // 合理價 < 股價 <= 昂貴價的股票數量
    pub highly_overvalued: i32,            // 股價 > 昂貴價的股票數量
    pub below_5_day_moving_average: i32,   // 股價 < 月線的股票數量
    pub above_5_day_moving_average: i32,   // 股價 >= 月線的股票數量
    pub below_20_day_moving_average: i32,  // 股價 < 月線的股票數量
    pub above_20_day_moving_average: i32,  // 股價 >= 月線的股票數量
    pub below_60_day_moving_average: i32,  // 股價 < 季線的股票數量
    pub above_60_day_moving_average: i32,  // 股價 >= 季線的股票數量
    pub below_120_day_moving_average: i32, // 股價 < 半年線的股票數量
    pub above_120_day_moving_average: i32, // 股價 >= 半年線的股票數量
    pub below_240_day_moving_average: i32, // 股價 < 年線的股票數量
    pub above_240_day_moving_average: i32, // 股價 >= 年線的股票數量
    pub stocks_up: i32,                    // 上漲的股票數量
    pub stocks_down: i32,                  // 下跌的股票數量
    pub stocks_unchanged: i32,             // 持平的股票數量
    pub created_at: DateTime<Local>,       // 記錄創建時間
    pub updated_at: DateTime<Local>,       // 記錄最後更新時間
}

impl DailyStockPriceStats {
    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO daily_stock_price_stats (
    date,
    stock_exchange_market_id,
    undervalued,
    fair_valued,
    overvalued,
    highly_overvalued,
    below_5_day_moving_average,
    above_5_day_moving_average,
    below_20_day_moving_average,
    above_20_day_moving_average,
    below_60_day_moving_average,
    above_60_day_moving_average,
    below_120_day_moving_average,
    above_120_day_moving_average,
    below_240_day_moving_average,
    above_240_day_moving_average,
    stocks_up,
    stocks_down,
    stocks_unchanged
)
WITH raw_data AS (
    -- 核心數據拉取：一次性獲取估值、報價與均線數據
    SELECT 
        s.stock_exchange_market_id as market_id,
        dq."ClosingPrice" as actual_close, 
        e.cheap, e.fair, e.expensive,
        dq."ChangeRange",
        dq."MovingAverage5", dq."MovingAverage20", dq."MovingAverage60",
        dq."MovingAverage120", dq."MovingAverage240"
    FROM stocks s
    JOIN estimate e ON s."SuspendListing" = FALSE AND s.stock_symbol = e.security_code
    JOIN "DailyQuotes" dq ON e.date = dq."Date" AND e.security_code = dq."stock_symbol"
    WHERE e.date = $1
)
SELECT
    $1 as date,
    COALESCE(market_id, 0) as stock_exchange_market_id,
    -- 估值分布統計
    COUNT(*) FILTER (WHERE actual_close <= cheap) as undervalued,
    COUNT(*) FILTER (WHERE actual_close > cheap AND actual_close <= fair) as fair_valued,
    COUNT(*) FILTER (WHERE actual_close > fair AND actual_close <= expensive) as overvalued,
    COUNT(*) FILTER (WHERE actual_close > expensive) as highly_overvalued,
    -- 均線位置統計
    COUNT(*) FILTER (WHERE actual_close <= "MovingAverage5") as b_ma5,
    COUNT(*) FILTER (WHERE actual_close > "MovingAverage5") as a_ma5,
    COUNT(*) FILTER (WHERE actual_close <= "MovingAverage20") as b_ma20,
    COUNT(*) FILTER (WHERE actual_close > "MovingAverage20") as a_ma20,
    COUNT(*) FILTER (WHERE actual_close <= "MovingAverage60") as b_ma60,
    COUNT(*) FILTER (WHERE actual_close > "MovingAverage60") as a_ma60,
    COUNT(*) FILTER (WHERE actual_close <= "MovingAverage120") as b_ma120,
    COUNT(*) FILTER (WHERE actual_close > "MovingAverage120") as a_ma120,
    COUNT(*) FILTER (WHERE actual_close <= "MovingAverage240") as b_ma240,
    COUNT(*) FILTER (WHERE actual_close > "MovingAverage240") as a_ma240,
    -- 當日漲跌統計
    COUNT(*) FILTER (WHERE "ChangeRange" > 0) as up,
    COUNT(*) FILTER (WHERE "ChangeRange" < 0) as down,
    COUNT(*) FILTER (WHERE "ChangeRange" = 0) as unchanged
FROM raw_data
-- 使用 GROUPING SETS 同時產生不分市場 (()) 與各市場分項 ((market_id)) 的統計結果
GROUP BY GROUPING SETS ((), (market_id))
ON CONFLICT (date, stock_exchange_market_id) DO UPDATE SET
    undervalued = EXCLUDED.undervalued,
    fair_valued = EXCLUDED.fair_valued,
    overvalued = EXCLUDED.overvalued,
    highly_overvalued = EXCLUDED.highly_overvalued,
    below_5_day_moving_average = EXCLUDED.below_5_day_moving_average,
    above_5_day_moving_average = EXCLUDED.above_5_day_moving_average,
    below_20_day_moving_average = EXCLUDED.below_20_day_moving_average,
    above_20_day_moving_average = EXCLUDED.above_20_day_moving_average,
    below_60_day_moving_average = EXCLUDED.below_60_day_moving_average,
    above_60_day_moving_average = EXCLUDED.above_60_day_moving_average,
    below_120_day_moving_average = EXCLUDED.below_120_day_moving_average,
    above_120_day_moving_average = EXCLUDED.above_120_day_moving_average,
    below_240_day_moving_average = EXCLUDED.below_240_day_moving_average,
    above_240_day_moving_average = EXCLUDED.above_240_day_moving_average,
    stocks_up = EXCLUDED.stocks_up,
    stocks_down = EXCLUDED.stocks_down,
    stocks_unchanged = EXCLUDED.stocks_unchanged,
    updated_at = CURRENT_TIMESTAMP;
"#;

        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to daily_stock_price_stats::upsert({}) from database",
            &date
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};
    use std::time::Duration;
    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 DailyStockPriceStats::upsert".to_string());

        // 開始日期與結束日期
        let start_date = NaiveDate::parse_from_str("2026-02-03", "%Y-%m-%d").unwrap();
        let end_date = NaiveDate::parse_from_str("2026-02-03", "%Y-%m-%d").unwrap();

        // 迴圈遍歷日期
        let mut current_date = start_date;
        while current_date <= end_date {
            logging::debug_file_async(format!("處理日期: {}", current_date));

            match DailyStockPriceStats::upsert(current_date, &mut None).await {
                Ok(r) => {
                    logging::debug_file_async(format!(
                        "DailyStockPriceStats::upsert({:?}) 成功: {:#?}",
                        current_date, r
                    ));
                }
                Err(why) => {
                    logging::debug_file_async(format!(
                        "DailyStockPriceStats::upsert({:?}) 失敗: {:?}",
                        current_date, why
                    ));
                }
            }

            // 日期加一天
            current_date += chrono::Duration::days(1);
        }

        logging::debug_file_async("結束 DailyStockPriceStats::upsert".to_string());
        // 每次迴圈暫停 0.5 秒
        sleep(Duration::from_millis(500)).await;
    }
}
