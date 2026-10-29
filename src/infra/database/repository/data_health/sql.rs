//! 資料健康檢查用的 SQL 與衍生表清單。

/// 每日收盤後會更新的衍生資料表，與各自的日期欄位。
///
/// 表名與欄位名無法用參數綁定，只能寫死在這裡；新增衍生表時同步加入。
pub(super) const DERIVED_TABLES: [(&str, &str); 15] = [
    ("estimate", "SELECT max(date) FROM estimate"),
    ("yield_rank", "SELECT max(date) FROM yield_rank"),
    (
        "daily_stock_price_stats",
        "SELECT max(date) FROM daily_stock_price_stats",
    ),
    (
        "last_daily_quotes",
        "SELECT max(date) FROM last_daily_quotes",
    ),
    ("stock_cagr", "SELECT max(date) FROM stock_cagr"),
    ("qfii_history", "SELECT max(date) FROM qfii_history"),
    (
        "daily_technical_signal",
        "SELECT max(date) FROM daily_technical_signal",
    ),
    (
        "daily_relative_strength",
        "SELECT max(date) FROM daily_relative_strength",
    ),
    ("chip_daily", "SELECT max(date) FROM chip_daily"),
    // 只抓持股、來源只有當天，停更就補不回來，要盡早發現。
    ("broker_flow", "SELECT max(date) FROM broker_flow"),
    (
        "daily_market_valuation",
        "SELECT max(date) FROM daily_market_valuation",
    ),
    // 17:30 ETF 專區每日更新：含息績效與清單頁快照（計算基準日都是最後一個有成交的交易日）。
    (
        "etf_performance",
        "SELECT max(calc_date) FROM etf_performance",
    ),
    ("etf_snapshot", "SELECT max(quote_date) FROM etf_snapshot"),
    // 基準日要再過 5 個交易日才有 5 日報酬，看「有 5 日報酬的最新基準日」。
    (
        "daily_forward_return",
        "SELECT max(date) FROM daily_forward_return WHERE return_5d IS NOT NULL",
    ),
    // 寫入未來報酬時同步重算的大盤彙總，口徑同上。
    (
        "daily_market_forward_return",
        "SELECT max(date) FROM daily_market_forward_return WHERE market_id = 0 AND n5 > 0",
    ),
];

/// 大盤未來報酬彙總與明細比對往回看幾天：涵蓋最長的 240 個交易日天期（約 350 天）。
pub(super) const MARKET_FORWARD_RETURN_DRIFT_DAYS: u64 = 400;

/// `$1` 起每個基準日，明細（`daily_forward_return` 連到 `stocks` 的列，與彙總同口徑）各天期的筆數
/// 和彙總表全市場那一列對不上的日期；任一邊沒有那一天也算。
///
/// 正常情況下寫入未來報酬時就會重算該日彙總；對不上代表有人直接改了明細、或重算失敗。
pub(super) const MARKET_FORWARD_RETURN_DRIFT_SQL: &str = r#"
WITH detail AS (
    SELECT r.date,
           count(r.return_5d) AS n5, count(r.return_20d) AS n20, count(r.return_60d) AS n60,
           count(r.return_120d) AS n120, count(r.return_240d) AS n240
    FROM daily_forward_return r
    JOIN stocks s ON s.stock_symbol = r.stock_symbol
    WHERE r.date >= $1
    GROUP BY r.date
),
summary AS (
    SELECT date, n5, n20, n60, n120, n240
    FROM daily_market_forward_return
    WHERE market_id = 0 AND date >= $1
)
SELECT coalesce(d.date, m.date) AS date
FROM detail d
FULL JOIN summary m ON m.date = d.date
WHERE (d.n5, d.n20, d.n60, d.n120, d.n240) IS DISTINCT FROM (m.n5, m.n20, m.n60, m.n120, m.n240)
ORDER BY 1
"#;

/// 短天期缺漏的列數占該基準日全部列數至少這個百分比才列出。
///
/// 回補範圍沒涵蓋到時整天都缺（接近 100%）；個股停牌、債券 ETF 不是每天成交，之後第 N 個
/// 交易日沒有報價，會造成零星缺值（2021-12-30 有 154 檔上櫃債券 ETF，約 8%），不算異常。
pub(super) const FORWARD_RETURN_GAP_MIN_PERCENT: i64 = 20;

/// 從最早有 5 日報酬的基準日起，「較短天期為空、較長天期有值」的列數占比達門檻的基準日。
///
/// 更早的基準日本來就只回補了長天期（例如只為了相對強度補 240 日報酬），不列入。
pub(super) const FORWARD_RETURN_GAPS_SQL: &str = r#"
WITH start AS (
    SELECT min(date) AS first_date FROM daily_forward_return WHERE return_5d IS NOT NULL
),
flagged AS (
    SELECT r.date,
           (r.return_5d IS NULL
            AND coalesce(r.return_20d, r.return_60d, r.return_120d, r.return_240d) IS NOT NULL)
           OR (r.return_20d IS NULL
               AND coalesce(r.return_60d, r.return_120d, r.return_240d) IS NOT NULL)
           OR (r.return_60d IS NULL AND coalesce(r.return_120d, r.return_240d) IS NOT NULL)
           OR (r.return_120d IS NULL AND r.return_240d IS NOT NULL) AS gap
    FROM daily_forward_return AS r, start
    WHERE r.date >= start.first_date
)
SELECT date, count(*) FILTER (WHERE gap)
FROM flagged
GROUP BY date
HAVING count(*) FILTER (WHERE gap) * 100 >= count(*) * $1
ORDER BY date
"#;

/// 檢查期間內有收盤價、前 400 天內已有至少 5 筆報價，5 日均線卻仍是 0 的列數。
pub(super) const MISSING_MOVING_AVERAGES_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes" AS d
WHERE d."Date" BETWEEN $1 AND $2
  AND d."ClosingPrice" > 0
  AND d."MovingAverage5" = 0
  AND (
      SELECT count(*)
      FROM (
          SELECT 1
          FROM "DailyQuotes" AS p
          WHERE p.stock_symbol = d.stock_symbol
            AND p."Date" <= d."Date"
            AND p."Date" >= d."Date" - 400
          LIMIT 5
      ) AS recent
  ) = 5
"#;

/// 漲跌幅與「漲跌 ÷ 參考價（收盤 − 漲跌）」不符的列數；漲跌為 0 時漲跌幅也必須是 0。
pub(super) const CHANGE_RANGE_MISMATCHES_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes"
WHERE "Date" BETWEEN $1 AND $2
  AND CASE
          WHEN "Change" = 0 OR "ClosingPrice" - "Change" <= 0 THEN "ChangeRange" <> 0
          ELSE abs("ChangeRange" - "Change" / ("ClosingPrice" - "Change") * 100) > 0.01
      END
"#;

/// `year`／`month`／`day` 與日期不符的列數（補值列曾漏填成 0）。
pub(super) const MISDATED_ROWS_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes"
WHERE "Date" BETWEEN $1 AND $2
  AND (year, month, day) IS DISTINCT FROM (
      date_part('year', "Date")::int,
      date_part('month', "Date")::int,
      date_part('day', "Date")::int
  )
"#;

/// 連續買賣超天數還沒計算的籌碼列數：每日入庫後的重算失敗時，排行、選股與回測會把它們當成 0。
pub(super) const CHIP_STREAK_MISSING_SQL: &str = r#"
SELECT count(*)
FROM chip_daily
WHERE "date" BETWEEN $1 AND $2
  AND (foreign_streak IS NULL OR trust_streak IS NULL OR dealer_streak IS NULL OR total_streak IS NULL)
"#;

/// 集保全部持股分級的最新資料日與該週列數，以及千張大戶週摘要最新一週 `holders_400` 是空值的檔數。
///
/// 兩張表由同一個週六排程寫入；只看各自最新一週，歷史週的 400 張欄位本來就有空值（分級資料
/// 2023-07 起才有）。兩張表的主鍵都以日期開頭，`max("date")` 走索引。
pub(super) const HOLDER_LEVEL_HEALTH_SQL: &str = r#"
WITH latest_level AS (SELECT max("date") AS date FROM holder_distribution_level),
     latest_holder AS (SELECT max("date") AS date FROM holder_distribution)
SELECT
    (SELECT date FROM latest_level),
    (SELECT count(*) FROM holder_distribution_level l JOIN latest_level m ON l."date" = m.date),
    (SELECT count(*) FROM holder_distribution h JOIN latest_holder m ON h."date" = m.date
     WHERE h.holders_400 IS NULL)
"#;

/// 近五年（不含今年）發放、除權息日仍是「尚未公布」的股利列數。
///
/// 更早的佔位列（1990 年代的配股）交易所已查不到實際日期，列出來也無從處理。
pub(super) const STALE_DIVIDEND_PLACEHOLDERS_SQL: &str = r#"
SELECT count(*)
FROM dividend
WHERE year < $1 AND year >= $1 - 5
  AND ("ex-dividend_date1" = '尚未公布' OR "ex-dividend_date2" = '尚未公布')
"#;

/// 年度合計列與各期明細加總不符的組數（只比對有明細的年度）。
pub(super) const DIVIDEND_TOTAL_MISMATCHES_SQL: &str = r#"
WITH details AS (
    SELECT security_code, year,
           sum(cash_dividend) AS cash_dividend,
           sum(stock_dividend) AS stock_dividend
    FROM dividend
    WHERE quarter <> ''
    GROUP BY security_code, year
)
SELECT count(*)
FROM dividend AS total
JOIN details USING (security_code, year)
WHERE total.quarter = ''
  AND (abs(total.cash_dividend - details.cash_dividend) > 0.0001
       OR abs(total.stock_dividend - details.stock_dividend) > 0.0001)
"#;

/// 每個交易日、每個市場有成交的個股（不含 ETF）中，本益比與股價淨值比為 0 的列數。
pub(super) const VALUATION_RATIO_ZEROS_SQL: &str = r#"
SELECT q."Date", s.stock_exchange_market_id,
       count(*),
       count(*) FILTER (WHERE q."PriceEarningRatio" = 0),
       count(*) FILTER (WHERE q."price-to-book_ratio" = 0)
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $1 AND $2
  AND s.stock_exchange_market_id IN (2, 4)
  AND q."ClosingPrice" > 0
  AND q.stock_symbol !~ '^00'
GROUP BY 1, 2
ORDER BY 1, 2
"#;

/// 每天的 ETF 淨值列數與正式淨值還沒回填的列數。
pub(super) const ETF_NAV_COUNTS_SQL: &str = r#"
SELECT date, count(*), count(*) FILTER (WHERE nav IS NULL)
FROM etf_daily_nav
WHERE date BETWEEN $1 AND $2
GROUP BY date
ORDER BY date
"#;

/// 清單頁上的 ETF 中，基本資料在 `$1` 之前就沒再更新的代號；還沒有基本資料的，
/// 只列出淨值在 `$1` 之前就出現（上市已超過一次週更）的，剛上市的等下一次週六更新。
pub(super) const STALE_ETF_PROFILES_SQL: &str = r#"
SELECT s.stock_symbol
FROM etf_snapshot AS s
LEFT JOIN etf_profile AS p ON p.stock_symbol = s.stock_symbol
WHERE CASE
          WHEN p.stock_symbol IS NULL THEN
              (SELECT min(n.date) FROM etf_daily_nav AS n WHERE n.stock_symbol = s.stock_symbol) < $1
          ELSE p.updated_at < $1::date
      END
ORDER BY s.stock_symbol
"#;
