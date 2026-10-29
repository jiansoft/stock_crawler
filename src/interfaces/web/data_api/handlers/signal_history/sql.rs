//! 訊號績效與個股訊號的 SQL：訊號篩選、籌碼連續買超與大盤基準的共用片段以 `macro_rules!`
//! 組合成 `&'static str` 常數，參數順序見各常數說明。

/// `$1`（含）之前最近一個有訊號的交易日；`$1` 為 `NULL` 時取最新。
pub(super) const LATEST_SIGNAL_DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_technical_signal WHERE $1::date IS NULL OR date <= $1
"#;

/// 訊號績效 SQL 的共用尾段：大盤基準（`bench`）與依 `sig(date, stock_symbol, indicator, kind)`
/// 彙總的各天期統計。`$1`～`$3` 為起日、迄日與市場 id（0 為上市櫃全部）。大盤基準讀每日彙總表
/// `daily_market_forward_return`（`sum / n` 與逐列 `AVG` 完全相同），不再每次掃三年的逐檔報酬。
///
/// `by_year` 版本以 `GROUPING SETS` 在同一次掃描算出整段期間（`year` 為 `NULL`）與各年度的統計；
/// 中位數無法由各年度合併，所以不另外跑兩次查詢。
macro_rules! performance_aggregate_sql {
    () => {
        performance_aggregate_sql!(@ "", "GROUP BY sig.indicator, sig.kind")
    };
    (by_year) => {
        performance_aggregate_sql!(@
            "\n    date_part('year', sig.date)::int AS year,",
            "GROUP BY GROUPING SETS ((sig.indicator, sig.kind), (sig.indicator, sig.kind, date_part('year', sig.date)::int))"
        )
    };
    (@ $year:literal, $group:literal) => {
        concat!(
            r#"bench AS (
    SELECT date,
           sum5 / NULLIF(n5, 0) AS b5, sum20 / NULLIF(n20, 0) AS b20, sum60 / NULLIF(n60, 0) AS b60,
           sum120 / NULLIF(n120, 0) AS b120, sum240 / NULLIF(n240, 0) AS b240
    FROM daily_market_forward_return
    WHERE date BETWEEN $1 AND $2
      AND market_id = $3
)
SELECT
    sig.indicator,
    sig.kind,"#,
            $year,
            r#"
    COUNT(*) AS occurrences,
    COUNT(r.return_5d) AS n5,
    AVG(r.return_5d)::float8 AS avg5,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_5d::float8) AS med5,
    AVG(CASE WHEN r.return_5d > 0 THEN 1.0 WHEN r.return_5d IS NOT NULL THEN 0.0 END)::float8 AS win5,
    AVG(r.return_5d - b.b5)::float8 AS excess5,
    AVG(CASE WHEN r.return_5d > b.b5 THEN 1.0 WHEN r.return_5d IS NOT NULL AND b.b5 IS NOT NULL THEN 0.0 END)::float8 AS beat5,
    COUNT(r.return_20d) AS n20,
    AVG(r.return_20d)::float8 AS avg20,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_20d::float8) AS med20,
    AVG(CASE WHEN r.return_20d > 0 THEN 1.0 WHEN r.return_20d IS NOT NULL THEN 0.0 END)::float8 AS win20,
    AVG(r.return_20d - b.b20)::float8 AS excess20,
    AVG(CASE WHEN r.return_20d > b.b20 THEN 1.0 WHEN r.return_20d IS NOT NULL AND b.b20 IS NOT NULL THEN 0.0 END)::float8 AS beat20,
    COUNT(r.return_60d) AS n60,
    AVG(r.return_60d)::float8 AS avg60,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_60d::float8) AS med60,
    AVG(CASE WHEN r.return_60d > 0 THEN 1.0 WHEN r.return_60d IS NOT NULL THEN 0.0 END)::float8 AS win60,
    AVG(r.return_60d - b.b60)::float8 AS excess60,
    AVG(CASE WHEN r.return_60d > b.b60 THEN 1.0 WHEN r.return_60d IS NOT NULL AND b.b60 IS NOT NULL THEN 0.0 END)::float8 AS beat60,
    COUNT(r.return_120d) AS n120,
    AVG(r.return_120d)::float8 AS avg120,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_120d::float8) AS med120,
    AVG(CASE WHEN r.return_120d > 0 THEN 1.0 WHEN r.return_120d IS NOT NULL THEN 0.0 END)::float8 AS win120,
    AVG(r.return_120d - b.b120)::float8 AS excess120,
    AVG(CASE WHEN r.return_120d > b.b120 THEN 1.0 WHEN r.return_120d IS NOT NULL AND b.b120 IS NOT NULL THEN 0.0 END)::float8 AS beat120,
    COUNT(r.return_240d) AS n240,
    AVG(r.return_240d)::float8 AS avg240,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_240d::float8) AS med240,
    AVG(CASE WHEN r.return_240d > 0 THEN 1.0 WHEN r.return_240d IS NOT NULL THEN 0.0 END)::float8 AS win240,
    AVG(r.return_240d - b.b240)::float8 AS excess240,
    AVG(CASE WHEN r.return_240d > b.b240 THEN 1.0 WHEN r.return_240d IS NOT NULL AND b.b240 IS NOT NULL THEN 0.0 END)::float8 AS beat240
FROM sig
LEFT JOIN daily_forward_return r ON r.date = sig.date AND r.stock_symbol = sig.stock_symbol
LEFT JOIN bench b ON b.date = sig.date
"#,
            $group,
            "\n"
        )
    };
}
pub(in crate::interfaces::web::data_api::handlers) use performance_aggregate_sql;

/// 訊號篩選的共用條件：`$1`／`$2` 起訖、`$3` 市場 id（0 為上市櫃全部）、`$4`／`$5` 來源與種類、
/// `$6` 產業名稱、`$7`／`$8` 訊號當天相對強度分數（`daily_relative_strength`，別名 `rs`）的上下限，
/// 有設時沒有分數的訊號不列入。
macro_rules! signal_filter_sql {
    () => {
        r#"
    WHERE t.date BETWEEN $1 AND $2
      AND ($3 = 0 OR s.stock_exchange_market_id = $3)
      AND ($4::text IS NULL OR t.indicator = $4)
      AND ($5::text IS NULL OR t.kind = $5)
      AND ($6::text IS NULL OR si.name = $6)
      AND ($7::smallint IS NULL OR rs.rs_score >= $7)
      AND ($8::smallint IS NULL OR rs.rs_score <= $8)"#
    };
}

/// 依 [`signal_filter_sql`] 篩出的訊號（`sig`）。
macro_rules! signal_cte {
    () => {
        concat!(
            r#"
WITH sig AS (
    SELECT t.date, t.stock_symbol, t.indicator, t.kind
    FROM daily_technical_signal t
    JOIN stocks s ON s.stock_symbol = t.stock_symbol
    LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
    LEFT JOIN daily_relative_strength rs ON rs.date = t.date AND rs.stock_symbol = t.stock_symbol"#,
            signal_filter_sql!(),
            r#"
),
"#
        )
    };
}

/// 各來源與種類的訊號在各天期的報酬統計，條件見 [`signal_filter_sql`]。大盤基準（`bench`）是
/// 同一天同市場全部股票的平均報酬，不受產業、RS 與籌碼篩選影響。
pub(super) const PERFORMANCE_SQL: &str = concat!(signal_cte!(), performance_aggregate_sql!());

/// [`PERFORMANCE_SQL`] 再加上各年度的統計（`year` 不為 `NULL` 的列）。
pub(super) const PERFORMANCE_BY_YEAR_SQL: &str =
    concat!(signal_cte!(), performance_aggregate_sql!(by_year));

/// [`signal_cte`] 再加籌碼條件：訊號當天外資（`$9`）／投信（`$10`）已連續買超至少 N 天，
/// `NULL` 為不篩選。連續天數與 `/market/institutional-flow` 相同，讀 `chip_daily` 預先算好的欄位。
/// 只在有設籌碼條件時使用。
macro_rules! chip_signal_cte {
    () => {
        concat!(
            "\nWITH",
            chip_streak_ctes!(),
            r#",
sig AS (
    SELECT t.date, t.stock_symbol, t.indicator, t.kind
    FROM daily_technical_signal t
    JOIN stocks s ON s.stock_symbol = t.stock_symbol
    JOIN chip c ON c."date" = t.date AND c.stock_symbol = t.stock_symbol
    LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
    LEFT JOIN daily_relative_strength rs ON rs.date = t.date AND rs.stock_symbol = t.stock_symbol"#,
            signal_filter_sql!(),
            r#"
      AND ($9::bigint IS NULL OR c.foreign_buy >= $9)
      AND ($10::bigint IS NULL OR c.trust_buy >= $10)
),
"#
        )
    };
}

/// 每檔股票每天外資（`foreign_buy`）、投信（`trust_buy`）已連續買超的天數（`chip`；賣超或沒有
/// 買賣為 0），讀 `$1`～`$2` 預先算好的連續天數；不含開頭的 `WITH` 與結尾的逗號，給需要籌碼條件的
/// SQL 共用。
macro_rules! chip_streak_ctes {
    () => {
        r#"
chip AS (
    SELECT stock_symbol, "date",
           GREATEST(COALESCE(foreign_streak, 0), 0) AS foreign_buy,
           GREATEST(COALESCE(trust_streak, 0), 0) AS trust_buy
    FROM chip_daily
    WHERE "date" BETWEEN $1 AND $2
)"#
    };
}
pub(in crate::interfaces::web::data_api::handlers) use chip_streak_ctes;

/// 有設籌碼條件時的訊號績效。
pub(super) const PERFORMANCE_WITH_CHIP_SQL: &str =
    concat!(chip_signal_cte!(), performance_aggregate_sql!());

/// [`PERFORMANCE_WITH_CHIP_SQL`] 再加上各年度的統計。
pub(super) const PERFORMANCE_WITH_CHIP_BY_YEAR_SQL: &str =
    concat!(chip_signal_cte!(), performance_aggregate_sql!(by_year));

/// 大盤基準的欄位：同市場全部股票各天期的樣本數、平均報酬與上漲比例，由每日彙總表
/// `daily_market_forward_return` 加總（區間平均 = Σsum ÷ Σn，與逐列 `AVG` 完全相同）。
macro_rules! benchmark_columns {
    () => {
        r#"
    COALESCE(SUM(r.n5), 0)::bigint AS n5,
    (SUM(r.sum5) / NULLIF(SUM(r.n5), 0))::float8 AS avg5,
    (SUM(r.win5) / NULLIF(SUM(r.n5), 0))::float8 AS win5,
    COALESCE(SUM(r.n20), 0)::bigint AS n20,
    (SUM(r.sum20) / NULLIF(SUM(r.n20), 0))::float8 AS avg20,
    (SUM(r.win20) / NULLIF(SUM(r.n20), 0))::float8 AS win20,
    COALESCE(SUM(r.n60), 0)::bigint AS n60,
    (SUM(r.sum60) / NULLIF(SUM(r.n60), 0))::float8 AS avg60,
    (SUM(r.win60) / NULLIF(SUM(r.n60), 0))::float8 AS win60,
    COALESCE(SUM(r.n120), 0)::bigint AS n120,
    (SUM(r.sum120) / NULLIF(SUM(r.n120), 0))::float8 AS avg120,
    (SUM(r.win120) / NULLIF(SUM(r.n120), 0))::float8 AS win120,
    COALESCE(SUM(r.n240), 0)::bigint AS n240,
    (SUM(r.sum240) / NULLIF(SUM(r.n240), 0))::float8 AS avg240,
    (SUM(r.win240) / NULLIF(SUM(r.n240), 0))::float8 AS win240
FROM daily_market_forward_return r
WHERE r.date BETWEEN $1 AND $2
  AND r.market_id = $3
"#
    };
}

/// 期間內同市場全部股票的報酬統計（大盤基準）。
pub(in crate::interfaces::web::data_api::handlers) const BENCHMARK_SQL: &str =
    concat!("SELECT", benchmark_columns!());

/// [`BENCHMARK_SQL`] 再加上各年度的基準；整段期間那一列的 `year` 為 `NULL`。空期間也會回整段那一列。
pub(super) const BENCHMARK_BY_YEAR_SQL: &str = concat!(
    "SELECT date_part('year', r.date)::int AS year,",
    benchmark_columns!(),
    "GROUP BY GROUPING SETS ((), (date_part('year', r.date)::int))\n"
);

/// [`BENCHMARK_SQL`] 依基準日的月份（1～12，跨年度合併）分組，附各月份涵蓋的年度數，由 1 月到 12 月。
pub(in crate::interfaces::web::data_api::handlers) const BENCHMARK_BY_MONTH_SQL: &str = concat!(
    "SELECT date_part('month', r.date)::int AS month,
",
    "    count(DISTINCT date_part('year', r.date))::bigint AS years,",
    benchmark_columns!(),
    "GROUP BY 1
ORDER BY 1
"
);

/// 個股在期間內的訊號，附當日報價與之後的報酬，由新到舊；`total` 為未套用 limit 的筆數。
pub(super) const STOCK_SIGNALS_SQL: &str = r#"
SELECT
    t.date,
    t.indicator,
    t.kind,
    t.description,
    q."ClosingPrice" AS closing_price,
    q."ChangeRange" AS change_range,
    r.return_5d,
    r.return_20d,
    r.return_60d,
    r.return_120d,
    r.return_240d,
    COUNT(*) OVER () AS total
FROM daily_technical_signal t
LEFT JOIN "DailyQuotes" q ON q.stock_symbol = t.stock_symbol AND q."Date" = t.date
LEFT JOIN daily_forward_return r ON r.stock_symbol = t.stock_symbol AND r.date = t.date
WHERE t.stock_symbol = $1
  AND t.date BETWEEN $2 AND $3
  AND ($4::text IS NULL OR t.indicator = $4)
  AND ($5::text IS NULL OR t.kind = $5)
ORDER BY t.date DESC, t.indicator, t.kind
LIMIT $6
"#;

/// `$1`～`$2` 期間內出現過的訊號來源與種類（`$3`／`$4` 為請求指定的來源與種類，`NULL` 為不限）；
/// 訊號績效依這個清單分組平行查詢（見 `fetch` 模組）。走主鍵的 index only scan，三年約 0.2 秒。
pub(super) const SIGNAL_COMBOS_SQL: &str = r#"
SELECT DISTINCT indicator, kind
FROM daily_technical_signal
WHERE date BETWEEN $1 AND $2
  AND ($3::text IS NULL OR indicator = $3)
  AND ($4::text IS NULL OR kind = $4)
ORDER BY indicator, kind
"#;
