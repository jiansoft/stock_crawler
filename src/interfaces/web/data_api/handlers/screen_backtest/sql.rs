//! 條件選股回測的 SQL：最新相對強度日與回測主體（有、無籌碼條件兩版）。

use super::super::signal_history::chip_streak_ctes;

#[cfg(doc)]
use super::{ETF_BENCHMARK, select_holdings};

/// `$1`（含）之前最近一個有相對強度的交易日；`$1` 為 `NULL` 時取最新。
pub(super) const LATEST_RS_DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_relative_strength WHERE $1::date IS NULL OR date <= $1
"#;

/// 回測 SQL 的主體。`$1`／`$2` 起訖、`$3` 市場 id（0 為上市櫃全部）、`$4` 持有天數、`$5` 每期候選檔數、
/// `$6` 產業、`$7`／`$8` 相對強度上下限、`$9`／`$10` 訊號來源與種類、`$11` 大盤濾網均線天數
/// （`NULL` 為不濾）、`$12` 續抱的相對強度門檻（`NULL` 為不設）、`$13` 市場廣度門檻（%，`NULL` 為不濾）；
/// 籌碼版另有 `$14`／`$15`。
///
/// - `days`：有相對強度的交易日（往前多讀 400 個日曆天，讓第一期也能回看訊號）。
/// - `rebal`：起日之後每隔 `$4` 個交易日的換股日；`window_start` 是含當天往前第 `$4` 個交易日，
///   訊號條件看 `window_start`～換股日。
/// - `bench`：同一天同市場全部母體股票的平均報酬；`samples` 為 0 代表還沒滿持有天數，不列入。
/// - `candidates`：符合條件的股票依相對強度由高到低的前 `$5` 檔與各自的報酬、產業編號；實際持股由
///   [`select_holdings`] 依上一期持股、`keep_rank` 與 `max_per_industry` 決定。
/// - `holdable`：相對強度不低於 `$12` 的母體股票，供上一期持股判斷能否續抱（不看其他選股條件）。
/// - `taiex`：加權指數收盤是否不低於含當天往前 `$11` 個交易日的平均；低於時 `market_ok` 為
///   false，這期空手。
/// - `breadth`：同市場站上 60 日均線的股票比例（`daily_stock_price_stats`）是否不低於 `$13`%；
///   低於時同樣空手。換股日沒有統計列視為通過（與加權指數缺資料相同）。
/// - `etf_return`：同一天買進 [`ETF_BENCHMARK`] 持有同樣天數的還原報酬，作為實際買得到的大盤基準。
macro_rules! backtest_sql {
    ($with:literal, $chip_join:literal, $chip_filter:literal) => {
        concat!(
            $with,
            r#"
days AS (
    SELECT date FROM daily_relative_strength
    WHERE date BETWEEN $1::date - 400 AND $2
    GROUP BY date
),
windowed AS (
    SELECT date, LAG(date, $4::int - 1, date) OVER (ORDER BY date) AS window_start
    FROM days
),
rebal AS (
    SELECT date, window_start
    FROM (
        SELECT date, window_start, ROW_NUMBER() OVER (ORDER BY date) - 1 AS i
        FROM windowed
        WHERE date >= $1
    ) AS numbered
    WHERE i % $4::int = 0
),
universe AS (
    SELECT rb.date, rb.window_start, rs.stock_symbol, rs.rs_score, s.stock_industry_id,
           CASE $4::int
               WHEN 5 THEN f.return_5d
               WHEN 20 THEN f.return_20d
               WHEN 60 THEN f.return_60d
               WHEN 120 THEN f.return_120d
               ELSE f.return_240d
           END AS ret
    FROM rebal rb
    JOIN daily_relative_strength rs ON rs.date = rb.date
    JOIN stocks s ON s.stock_symbol = rs.stock_symbol
    LEFT JOIN daily_forward_return f ON f.date = rb.date AND f.stock_symbol = rs.stock_symbol
    WHERE ($3 = 0 OR s.stock_exchange_market_id = $3)
),
bench AS (
    SELECT date, AVG(ret)::float8 AS avg_ret, COUNT(ret) AS samples
    FROM universe
    GROUP BY date
),
picks AS (
    SELECT u.date, u.stock_symbol, u.ret, u.stock_industry_id,
           ROW_NUMBER() OVER (PARTITION BY u.date ORDER BY u.rs_score DESC, u.stock_symbol) AS pick_rank
    FROM universe u
    LEFT JOIN stock_industry si ON si.stock_industry_id = u.stock_industry_id"#,
            $chip_join,
            r#"
    WHERE ($6::text IS NULL OR si.name = $6)
      AND ($7::smallint IS NULL OR u.rs_score >= $7)
      AND ($8::smallint IS NULL OR u.rs_score <= $8)
      AND (($9::text IS NULL AND $10::text IS NULL) OR EXISTS (
          SELECT 1 FROM daily_technical_signal t
          WHERE t.stock_symbol = u.stock_symbol
            AND t.date BETWEEN u.window_start AND u.date
            AND ($9::text IS NULL OR t.indicator = $9)
            AND ($10::text IS NULL OR t.kind = $10)
      ))"#,
            $chip_filter,
            r#"
),
candidates AS (
    SELECT date,
           array_agg(stock_symbol::text ORDER BY pick_rank) AS symbols,
           array_agg(ret::float8 ORDER BY pick_rank) AS returns,
           array_agg(stock_industry_id::int4 ORDER BY pick_rank) AS industries
    FROM picks
    WHERE pick_rank <= $5
    GROUP BY date
),
holdable AS (
    SELECT date,
           array_agg(stock_symbol::text ORDER BY rs_score DESC, stock_symbol) AS symbols,
           array_agg(ret::float8 ORDER BY rs_score DESC, stock_symbol) AS returns,
           array_agg(stock_industry_id::int4 ORDER BY rs_score DESC, stock_symbol) AS industries
    FROM universe
    WHERE $12::smallint IS NOT NULL AND rs_score >= $12
    GROUP BY date
),
taiex AS (
    SELECT "date",
           "index" >= AVG("index") OVER (
               ORDER BY "date"
               ROWS BETWEEN (COALESCE($11::int, 1) - 1) PRECEDING AND CURRENT ROW
           ) AS above
    FROM "index"
    WHERE category = 'TAIEX' AND "date" BETWEEN $1::date - 400 AND $2
),
breadth AS (
    SELECT date,
           above_60_day_moving_average * 100
               >= $13::int * NULLIF(above_60_day_moving_average + below_60_day_moving_average, 0) AS above
    FROM daily_stock_price_stats
    WHERE stock_exchange_market_id = $3 AND date BETWEEN $1 AND $2
)
SELECT b.date,
       ($11::int IS NULL OR t.above IS NOT FALSE)
           AND ($13::int IS NULL OR br.above IS NOT FALSE) AS market_ok,
       b.avg_ret AS benchmark_return,
       (CASE $4::int
            WHEN 5 THEN e.return_5d
            WHEN 20 THEN e.return_20d
            WHEN 60 THEN e.return_60d
            WHEN 120 THEN e.return_120d
            ELSE e.return_240d
        END)::float8 AS etf_return,
       COALESCE(c.symbols, ARRAY[]::text[]) AS candidates,
       COALESCE(c.returns, ARRAY[]::float8[]) AS candidate_returns,
       COALESCE(c.industries, ARRAY[]::int4[]) AS candidate_industries,
       COALESCE(h.symbols, ARRAY[]::text[]) AS hold_symbols,
       COALESCE(h.returns, ARRAY[]::float8[]) AS hold_returns,
       COALESCE(h.industries, ARRAY[]::int4[]) AS hold_industries
FROM bench b
LEFT JOIN candidates c ON c.date = b.date
LEFT JOIN holdable h ON h.date = b.date
LEFT JOIN daily_forward_return e ON e.date = b.date AND e.stock_symbol = '0050'
LEFT JOIN taiex t ON t."date" = b.date
LEFT JOIN breadth br ON br.date = b.date
WHERE b.samples > 0
ORDER BY b.date
"#
        )
    };
}

/// 沒有籌碼條件的回測。
pub(super) const BACKTEST_SQL: &str = backtest_sql!("\nWITH", "", "");

/// 有籌碼條件的回測：換股當天外資（`$14`）／投信（`$15`）已連續買超至少 N 天，`NULL` 為不篩選。
pub(super) const BACKTEST_WITH_CHIP_SQL: &str = concat!(
    "\nWITH",
    chip_streak_ctes!(),
    ",",
    backtest_sql!(
        "",
        "\n    JOIN chip c ON c.\"date\" = u.date AND c.stock_symbol = u.stock_symbol",
        "\n      AND ($14::bigint IS NULL OR c.foreign_buy >= $14)\n      AND ($15::bigint IS NULL OR c.trust_buy >= $15)"
    )
);
