use crate::database;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use serde_derive::{Deserialize, Serialize};
use sqlx::postgres::PgQueryResult;
use sqlx::{FromRow, Type};

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
    pub async fn upsert(date: NaiveDate) -> Result<PgQueryResult> {
        let sql = r#"
WITH cte AS (
    SELECT e.security_code, e.date, e.closing_price,
           e.cheap, e.fair, e.expensive,
           dq."ClosingPrice", dq."ChangeRange",
           dq."MovingAverage5", dq."MovingAverage20", dq."MovingAverage60",
           dq."MovingAverage120", dq."MovingAverage240",
           s.stock_exchange_market_id
    FROM stocks s
    INNER JOIN estimate e ON s."SuspendListing" = false AND s.stock_symbol = e.security_code
    INNER JOIN "DailyQuotes" dq ON e.date = dq."Date" AND e.security_code = dq."SecurityCode"
    WHERE e.date = $1
),
stats AS (
    SELECT
        date,
        CASE
            WHEN closing_price <= cheap THEN 'undervalued'
            WHEN closing_price > cheap AND closing_price <= fair THEN 'fair_valued'
            WHEN closing_price > fair AND closing_price <= expensive THEN 'overvalued'
            WHEN closing_price > expensive THEN  'highly_overvalued'
        END AS valuation_category,
        CASE
            WHEN closing_price <= "MovingAverage5" THEN 'below_week_ma'
            WHEN closing_price > "MovingAverage5" THEN 'above_week_ma'
        END AS ma5_category,
        CASE
            WHEN closing_price <= "MovingAverage20" THEN 'below_month_ma'
            WHEN closing_price > "MovingAverage20" THEN 'above_month_ma'
        END AS ma20_category,
        CASE
            WHEN closing_price <= "MovingAverage60" THEN 'below_quarter_ma'
            WHEN closing_price > "MovingAverage60" THEN 'above_quarter_ma'
        END AS ma60_category,
        CASE
            WHEN closing_price <= "MovingAverage120" THEN 'below_half_year_ma'
            WHEN closing_price > "MovingAverage120" THEN 'above_half_year_ma'
        END AS ma120_category,
        CASE
            WHEN closing_price <= "MovingAverage240" THEN 'below_year_ma'
            WHEN closing_price > "MovingAverage240" THEN 'above_year_ma'
        END AS ma240_category,
         CASE
            WHEN "ChangeRange" > 0 THEN 'up'
            WHEN "ChangeRange" < 0 THEN 'down'
            WHEN "ChangeRange" = 0 THEN 'unchanged'
        END AS change_category,
        stock_exchange_market_id
    FROM cte
),
final_stats AS (
    SELECT
        date,
        market,
        COUNT(*) FILTER (WHERE valuation_category = 'undervalued') AS undervalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'fair_valued') AS fair_valued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'overvalued') AS overvalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'highly_overvalued') AS highly_overvalued_count,
        COUNT(*) FILTER (WHERE ma5_category = 'below_week_ma') AS below_week_ma_count,
        COUNT(*) FILTER (WHERE ma5_category = 'above_week_ma') AS above_week_ma_count,
        COUNT(*) FILTER (WHERE ma20_category = 'below_month_ma') AS below_month_ma_count,
        COUNT(*) FILTER (WHERE ma20_category = 'above_month_ma') AS above_month_ma_count,
        COUNT(*) FILTER (WHERE ma60_category = 'below_quarter_ma') AS below_quarter_ma_count,
        COUNT(*) FILTER (WHERE ma60_category = 'above_quarter_ma') AS above_quarter_ma_count,
        COUNT(*) FILTER (WHERE ma120_category = 'below_half_year_ma') AS below_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma120_category = 'above_half_year_ma') AS above_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'below_year_ma') AS below_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'above_year_ma') AS above_year_ma_count,
        COUNT(*) FILTER (WHERE change_category = 'up') AS up_count,
        COUNT(*) FILTER (WHERE change_category = 'down') AS down_count,
        COUNT(*) FILTER (WHERE change_category = 'unchanged') AS unchanged_count
    FROM (
        SELECT 0 AS market, * FROM stats
        UNION ALL
        SELECT 2 AS market, * FROM stats WHERE stock_exchange_market_id = 2
        UNION ALL
        SELECT 4 AS market, * FROM stats WHERE stock_exchange_market_id = 4
    ) subquery
    GROUP BY date,market
)
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
SELECT
    date,
    market,
    undervalued_count,
    fair_valued_count,
    overvalued_count,
    highly_overvalued_count,
    below_week_ma_count,
    above_week_ma_count,
    below_month_ma_count,
    above_month_ma_count,
    below_quarter_ma_count,
    above_quarter_ma_count,
    below_half_year_ma_count,
    above_half_year_ma_count,
    below_year_ma_count,
    above_year_ma_count,
    up_count,
    down_count,
    unchanged_count
FROM final_stats
ON CONFLICT (date, stock_exchange_market_id) DO UPDATE SET
    undervalued = EXCLUDED.undervalued,
    fair_valued = EXCLUDED.fair_valued,
    overvalued = EXCLUDED.overvalued,
    highly_overvalued = EXCLUDED.highly_overvalued,
    below_5_day_moving_average = EXCLUDED.below_5_day_moving_average,
    above_5_day_moving_average = EXCLUDED.above_5_day_moving_average,
    below_20_day_moving_average = EXCLUDED.below_20_day_moving_average,
    above_20_day_moving_average = EXCLUDED.above_20_day_moving_average,
    below_60_day_moving_average = EXCLUDED.below_20_day_moving_average,
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

        sqlx::query(sql)
            .bind(date)
            .execute(database::get_connection())
            .await
            .map_err(|why| anyhow!("Failed to upsert() from database\nsql:{}\n{:?}", sql, why,))
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
        let start_date = NaiveDate::parse_from_str("2021-08-25", "%Y-%m-%d").unwrap();
        let end_date = NaiveDate::parse_from_str("2024-10-01", "%Y-%m-%d").unwrap();

        // 迴圈遍歷日期
        let mut current_date = start_date;
        while current_date <= end_date {
            logging::debug_file_async(format!("處理日期: {}", current_date));

            match DailyStockPriceStats::upsert(current_date).await {
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
