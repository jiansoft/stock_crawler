use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction, Type, postgres::PgQueryResult};

use crate::infra::database;

/// 每日全市場估值與技術面分布統計。
///
/// 同一天通常會有多筆資料：
/// - `stock_exchange_market_id = 0`：上市＋上櫃合併統計（不含興櫃與公開發行，
///   與 Data API 各排行、選股的 `all` 口徑一致）
/// - 其他 id：各市場分項統計（例如 TWSE、TPEx、興櫃）
#[derive(Debug, Serialize, Deserialize, Type, FromRow)]
pub struct DailyStockPriceStats {
    /// 統計日期。
    pub date: NaiveDate,
    /// 市場類型（TWSE: 2、TPEx: 4、全部市場: 0）。
    pub stock_exchange_market_id: i32,
    /// 股價 <= 便宜價的股票數量。
    pub undervalued: i32,
    /// 便宜價 < 股價 <= 合理價的股票數量。
    pub fair_valued: i32,
    /// 合理價 < 股價 <= 昂貴價的股票數量。
    pub overvalued: i32,
    /// 股價 > 昂貴價的股票數量。
    pub highly_overvalued: i32,
    /// 股價 <= 5 日均線的股票數量。
    pub below_5_day_moving_average: i32,
    /// 股價 > 5 日均線的股票數量。
    pub above_5_day_moving_average: i32,
    /// 股價 <= 20 日均線的股票數量。
    pub below_20_day_moving_average: i32,
    /// 股價 > 20 日均線的股票數量。
    pub above_20_day_moving_average: i32,
    /// 股價 <= 60 日均線的股票數量。
    pub below_60_day_moving_average: i32,
    /// 股價 > 60 日均線的股票數量。
    pub above_60_day_moving_average: i32,
    /// 股價 <= 120 日均線的股票數量。
    pub below_120_day_moving_average: i32,
    /// 股價 > 120 日均線的股票數量。
    pub above_120_day_moving_average: i32,
    /// 股價 <= 240 日均線的股票數量。
    pub below_240_day_moving_average: i32,
    /// 股價 > 240 日均線的股票數量。
    pub above_240_day_moving_average: i32,
    /// 當日上漲家數。
    pub stocks_up: i32,
    /// 當日下跌家數。
    pub stocks_down: i32,
    /// 當日平盤家數。
    pub stocks_unchanged: i32,
    /// 當日最高價創 52 週新高的家數。
    pub new_52_week_high: i32,
    /// 當日最低價創 52 週新低的家數。
    pub new_52_week_low: i32,
    /// 建立時間。
    pub created_at: DateTime<Local>,
    /// 最後更新時間。
    pub updated_at: DateTime<Local>,
}

impl DailyStockPriceStats {
    /// 產生或更新指定日期的股價統計資料。
    ///
    /// 此方法會從 `stocks`、`estimate`、`DailyQuotes` 彙整當日數據，
    /// 計算估值分布（便宜/合理/昂貴/昂貴以上）、
    /// 均線相對位置（5/20/60/120/240 日）、漲跌家數與 52 週新高／新低家數，
    /// 並同時寫入「上市＋上櫃合併」（`stock_exchange_market_id = 0`）與各市場的統計列。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤；若呼叫端有提供 transaction，
    /// 是否回滾由呼叫端控制。
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
    stocks_unchanged,
    new_52_week_high,
    new_52_week_low
)
WITH raw_data AS (
    -- 核心數據拉取
    SELECT 
        s.stock_exchange_market_id as market_id,
        dq."ClosingPrice" as actual_close, 
        e.cheap, e.fair, e.expensive,
        dq."ChangeRange",
        dq."MovingAverage5", dq."MovingAverage20", dq."MovingAverage60",
        dq."MovingAverage120", dq."MovingAverage240",
        dq."HighestPrice" as day_high, dq."LowestPrice" as day_low,
        w.prior_high, w.prior_low, w.prior_days
    FROM stocks s
    JOIN estimate e ON s."SuspendListing" = FALSE AND s.stock_symbol = e.security_code
    JOIN "DailyQuotes" dq ON e.date = dq."Date" AND e.security_code = dq."stock_symbol"
    -- 前 52 週（364 天、不含當日）的最高／最低價，走 (stock_symbol, "Date" DESC) 覆蓋索引
    CROSS JOIN LATERAL (
        SELECT
            MAX(p."HighestPrice") as prior_high,
            MIN(p."LowestPrice") FILTER (WHERE p."LowestPrice" > 0) as prior_low,
            COUNT(*) as prior_days
        FROM "DailyQuotes" p
        WHERE p.stock_symbol = dq.stock_symbol
          AND p."Date" < dq."Date"
          AND p."Date" >= dq."Date" - 364
          AND p."ClosingPrice" > 0
    ) w
    WHERE e.date = $1
),
market_metrics AS (
    -- 先按市場 ID 聚合
    SELECT
        market_id,
        COUNT(*) FILTER (WHERE actual_close <= cheap) as undervalued,
        COUNT(*) FILTER (WHERE actual_close > cheap AND actual_close <= fair) as fair_valued,
        COUNT(*) FILTER (WHERE actual_close > fair AND actual_close <= expensive) as overvalued,
        COUNT(*) FILTER (WHERE actual_close > expensive) as highly_overvalued,
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
        COUNT(*) FILTER (WHERE "ChangeRange" > 0) as up,
        COUNT(*) FILTER (WHERE "ChangeRange" < 0) as down,
        COUNT(*) FILTER (WHERE "ChangeRange" = 0) as unchanged,
        -- 前 52 週不足 200 個交易日（新上市）不算，避免把短歷史的高低點當成新高新低
        COUNT(*) FILTER (WHERE prior_days >= 200 AND day_high > prior_high) as new_high,
        COUNT(*) FILTER (WHERE prior_days >= 200 AND day_low > 0 AND day_low < prior_low) as new_low
    FROM raw_data
    GROUP BY market_id
),
final_set AS (
    -- 1. 產生上市＋上櫃合併 (ID = 0)；興櫃 (5) 與公開發行等其他市場只有自己的分項列
    SELECT
        0 as stock_exchange_market_id,
        COALESCE(SUM(undervalued), 0)::int as undervalued,
        COALESCE(SUM(fair_valued), 0)::int as fair_valued, 
        COALESCE(SUM(overvalued), 0)::int as overvalued,
        COALESCE(SUM(highly_overvalued), 0)::int as highly_overvalued,
        COALESCE(SUM(b_ma5), 0)::int as b_ma5,
        COALESCE(SUM(a_ma5), 0)::int as a_ma5,
        COALESCE(SUM(b_ma20), 0)::int as b_ma20,
        COALESCE(SUM(a_ma20), 0)::int as a_ma20,
        COALESCE(SUM(b_ma60), 0)::int as b_ma60,
        COALESCE(SUM(a_ma60), 0)::int as a_ma60,
        COALESCE(SUM(b_ma120), 0)::int as b_ma120,
        COALESCE(SUM(a_ma120), 0)::int as a_ma120,
        COALESCE(SUM(b_ma240), 0)::int as b_ma240,
        COALESCE(SUM(a_ma240), 0)::int as a_ma240,
        COALESCE(SUM(up), 0)::int as up,
        COALESCE(SUM(down), 0)::int as down,
        COALESCE(SUM(unchanged), 0)::int as unchanged,
        COALESCE(SUM(new_high), 0)::int as new_high,
        COALESCE(SUM(new_low), 0)::int as new_low
    FROM market_metrics
    WHERE market_id IN (2, 4)
    UNION ALL
    -- 2. 產生各市場分類 (排除 ID 0 以免與總計衝突)
    SELECT
        market_id as stock_exchange_market_id,
        undervalued, fair_valued, overvalued, highly_overvalued,
        b_ma5, a_ma5, b_ma20, a_ma20, b_ma60, a_ma60,
        b_ma120, a_ma120, b_ma240, a_ma240,
        up, down, unchanged, new_high, new_low
    FROM market_metrics
    WHERE market_id != 0
)
SELECT
    $1 as date,
    stock_exchange_market_id,
    undervalued, fair_valued, overvalued, highly_overvalued,
    b_ma5, a_ma5, b_ma20, a_ma20, b_ma60, a_ma60,
    b_ma120, a_ma120, b_ma240, a_ma240,
    up, down, unchanged, new_high, new_low
FROM final_set
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
    new_52_week_high = EXCLUDED.new_52_week_high,
    new_52_week_low = EXCLUDED.new_52_week_low,
    updated_at = CURRENT_TIMESTAMP;
"#;

        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to daily_stock_price_stats::upsert({}) from database",
            date
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;
    use std::time::Duration;
    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 DailyStockPriceStats::upsert");

        // 開始日期與結束日期
        let start_date = NaiveDate::parse_from_str("2026-02-03", "%Y-%m-%d").unwrap();
        let end_date = NaiveDate::parse_from_str("2026-02-03", "%Y-%m-%d").unwrap();

        // 迴圈遍歷日期
        let mut current_date = start_date;
        while current_date <= end_date {
            tracing::debug!("處理日期: {}", current_date);

            match DailyStockPriceStats::upsert(current_date, &mut None).await {
                Ok(r) => {
                    tracing::debug!(
                        "DailyStockPriceStats::upsert({:?}) 成功: {:#?}",
                        current_date,
                        r
                    );
                }
                Err(why) => {
                    tracing::debug!(
                        "DailyStockPriceStats::upsert({:?}) 失敗: {:?}",
                        current_date,
                        why
                    );
                }
            }

            // 日期加一天
            current_date += chrono::Duration::days(1);
        }

        tracing::debug!("結束 DailyStockPriceStats::upsert");
        // 每次迴圈暫停 0.5 秒
        sleep(Duration::from_millis(500)).await;
    }
    /// 52 週新高／新低：79979HA 創高、79979HB 創低、79979HC 歷史不足 200 日不計；
    /// 上市列與 all 列都只有 1 檔新高、1 檔新低；興櫃 79979HD 創高只算在興櫃列。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn upsert_counts_52_week_highs_and_lows() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 upsert_counts_52_week_highs_and_lows：無資料庫連接");
            return;
        }
        let date = NaiveDate::from_ymd_opt(1991, 6, 28).expect("固定日期合法");
        let symbols = ["79979HA", "79979HB", "79979HC", "79979HD"];
        let cleanup = || async {
            for sql in [
                r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
                "DELETE FROM estimate WHERE security_code = ANY($1)",
                "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
            ] {
                let _ = sqlx::query(sql).bind(symbols).execute(pool).await;
            }
            let _ = sqlx::query("DELETE FROM daily_stock_price_stats WHERE date = $1")
                .bind(date)
                .execute(pool)
                .await;
        };
        cleanup().await;

        // (代號, 市場, 前期天數, 當日最高, 當日最低)；前期每日最高 100、最低 90。
        // 79979HD 是興櫃：只計入自己的分項列，不計入 all（id 0）。
        for (symbol, market_id, prior_days, high, low) in [
            ("79979HA", 2, 250, 105, 95),
            ("79979HB", 2, 250, 95, 85),
            ("79979HC", 2, 50, 200, 150),
            ("79979HD", 5, 250, 105, 95),
        ] {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
                   VALUES ($1, $1, $1, $2, 1)"#,
            )
            .bind(symbol)
            .bind(market_id)
            .execute(pool)
            .await
            .expect("種入 stocks");
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice")
                   SELECT $2::date - g, $1, 95, 100, 90 FROM generate_series(1, $3) g
                   UNION ALL SELECT $2::date, $1, $4, $4, $5"#,
            )
            .bind(symbol)
            .bind(date)
            .bind(prior_days)
            .bind(high)
            .bind(low)
            .execute(pool)
            .await
            .expect("種入日報價");
            sqlx::query("INSERT INTO estimate (security_code, date) VALUES ($1, $2)")
                .bind(symbol)
                .bind(date)
                .execute(pool)
                .await
                .expect("種入 estimate");
        }

        DailyStockPriceStats::upsert(date, &mut None)
            .await
            .expect("upsert 成功");
        let rows: Vec<(i32, i32, i32)> = sqlx::query_as(
            "SELECT stock_exchange_market_id, new_52_week_high, new_52_week_low
             FROM daily_stock_price_stats WHERE date = $1 ORDER BY 1",
        )
        .bind(date)
        .fetch_all(pool)
        .await
        .expect("讀回統計");
        assert_eq!(rows, [(0, 1, 1), (2, 1, 1), (5, 1, 0)]);

        cleanup().await;
    }
}
