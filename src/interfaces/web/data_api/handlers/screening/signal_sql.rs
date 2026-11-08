//! 條件選股先由 handler 算好、再以平行陣列左接進主查詢的兩個輔助 SQL：千張大戶持股變化與近期區間突破。

/// 查詢日（`$1`，不含）以前最近一個集保資料週，相對往前第 `$2` 週的千張大戶持股比例變化（百分點），
/// 每檔一列；最近一週距查詢日超過 14 天時沒有資料。集保週五的資料週六才公布，只用早於查詢日的週，
/// 與回測（`holder_rebal`）同一規則。
pub(super) const MAJOR_CHANGES_SQL: &str = r#"
WITH weeks AS (
    SELECT "date", ROW_NUMBER() OVER (ORDER BY "date" DESC) - 1 AS back
    FROM (
        SELECT DISTINCT "date" FROM holder_distribution
        WHERE "date" < $1
        ORDER BY "date" DESC
        LIMIT $2::int + 1
    ) recent
)
SELECT cur.stock_symbol::text AS stock_symbol, cur.major_percent - base.major_percent AS major_change
FROM weeks cw
JOIN weeks bw ON bw.back = $2::int
JOIN holder_distribution cur ON cur."date" = cw."date"
JOIN holder_distribution base ON base."date" = bw."date" AND base.stock_symbol = cur.stock_symbol
WHERE cw.back = 0 AND cw."date" >= $1::date - 14
"#;

/// 查詢日（`$1`，含）以前最近 `$2` 個交易日（以有技術訊號的日子為交易日、往前最多看 45 個日曆天）出現
/// 區間突破的股票，同一檔只取最近一次；突破之後到查詢日出現假突破（跌回突破價）的不列入。
pub(super) const RANGE_BREAKOUTS_SQL: &str = r#"
SELECT DISTINCT ON (b.stock_symbol) b.stock_symbol::text, b.date, b.description
FROM daily_technical_signal b
WHERE b.indicator = 'range_breakout' AND b.kind = 'breakout'
  AND b.date IN (
      SELECT date FROM (
          SELECT DISTINCT date FROM daily_technical_signal
          WHERE date <= $1 AND date >= $1::date - 45
      ) AS days
      ORDER BY date DESC
      LIMIT $2
  )
  AND NOT EXISTS (
      SELECT 1 FROM daily_technical_signal f
      WHERE f.stock_symbol = b.stock_symbol AND f.indicator = 'range_breakout'
        AND f.kind = 'failed' AND f.date > b.date AND f.date <= $1
  )
ORDER BY b.stock_symbol, b.date DESC
"#;
