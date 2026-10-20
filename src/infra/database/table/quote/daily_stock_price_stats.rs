use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction, Type, postgres::PgQueryResult};

use crate::infra::database;

/// 寫入語句開頭：欄位清單與 `raw_data` CTE 起頭；後面接各自的 `raw_data` 內容。
macro_rules! stats_insert_head {
    () => {
        r#"
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
"#
    };
}

/// 由 `raw_data` 聚合出各市場與上市＋上櫃合併列的共用段落；後面接衝突處理。
macro_rules! stats_aggregate_tail {
    () => {
        r#"
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
"#
    };
}

/// 每日收盤後統計的 `raw_data`：只算當天有估價（`estimate`）且目前未下市的股票。
macro_rules! stats_estimate_raw_data {
    () => {
        r#"    -- 核心數據拉取
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
"#
    };
}

/// 每日收盤後統計：同一天重算時覆寫所有欄位。
const UPSERT_SQL: &str = concat!(
    stats_insert_head!(),
    stats_estimate_raw_data!(),
    stats_aggregate_tail!(),
    r#"ON CONFLICT (date, stock_exchange_market_id) DO UPDATE SET
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
"#
);

/// 歷史市場廣度回補：`estimate` 只有 2022-02-17 起的資料，更早的日子改從 `DailyQuotes` 取當天有日線的
/// 上市櫃股票（排除 `00` 開頭的 ETF／ETN，與 `estimate` 的範圍一致）；不看目前是否已下市，
/// 避免只留下存活到今天的股票。估值分布四欄沒有估價可比，一律為 0。
/// 已有的列一律保留（`DO NOTHING`），不會蓋掉收盤後依估價算出的統計。
const BACKFILL_BREADTH_SQL: &str = concat!(
    stats_insert_head!(),
    r#"
    SELECT
        s.stock_exchange_market_id as market_id,
        dq."ClosingPrice" as actual_close,
        NULL::numeric as cheap, NULL::numeric as fair, NULL::numeric as expensive,
        dq."ChangeRange",
        dq."MovingAverage5", dq."MovingAverage20", dq."MovingAverage60",
        dq."MovingAverage120", dq."MovingAverage240",
        dq."HighestPrice" as day_high, dq."LowestPrice" as day_low,
        w.prior_high, w.prior_low, w.prior_days
    FROM "DailyQuotes" dq
    JOIN stocks s ON s.stock_symbol = dq.stock_symbol
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
    WHERE dq."Date" = $1
      AND dq.stock_symbol NOT LIKE '00%'
"#,
    stats_aggregate_tail!(),
    // 合併列（id 0）是對空集合 SUM，非交易日也會產生一列全 0；當天沒有日線就整句不寫。
    "WHERE EXISTS (SELECT 1 FROM raw_data)
ON CONFLICT (date, stock_exchange_market_id) DO NOTHING;"
);

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
        let query = sqlx::query(UPSERT_SQL).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.context(format!(
            "Failed to daily_stock_price_stats::upsert({}) from database",
            date
        ))
    }

    /// 回補指定日期的市場廣度（均線上下、漲跌家數、52 週新高／新低），只補沒有統計列的日子。
    ///
    /// 給 `estimate` 還沒有資料的歷史日期用（2022-02-17 以前）：改從 `DailyQuotes` 取當天有日線的
    /// 上市櫃股票（不含 ETF／ETN、含後來下市的股票），估值分布四欄為 0。該日已有統計列時不動，
    /// 非交易日沒有日線也不會寫入。回傳新寫入的列數。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn backfill_breadth(date: NaiveDate) -> Result<u64> {
        let result = sqlx::query(BACKFILL_BREADTH_SQL)
            .bind(date)
            .execute(database::get_connection())
            .await
            .with_context(|| format!("daily_stock_price_stats::backfill_breadth({date}) 失敗"))?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests;
