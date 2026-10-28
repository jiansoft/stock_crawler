//! 回測 SQL 的 `filtered` CTE：符合流動性以外條件的股票（營收、財報、估值、大戶、籌碼）。

/// 展開 `filtered` CTE（含結尾的右括號）；`$chip_columns`／`$chip_join`／`$chip_filter` 為籌碼版的欄位、
/// JOIN 與條件（無籌碼版傳空字串與 `NULL` 欄位）。各段子查詢的規則見 [`super::BACKTEST_SQL`] 的說明。
macro_rules! filtered_cte {
    ($chip_columns:literal, $chip_join:literal, $chip_filter:literal) => {
        concat!(
            r#"filtered AS (
    SELECT u.date, u.liquidity_start, u.stock_symbol, u.rs_score, u.rs_rank, u.ret, u.stock_industry_id,
           rv.revenue_yoy::float8 AS revenue_yoy, ym.operating_margin, qv.pe, qv.pb,"#,
            $chip_columns,
            r#"
    FROM universe u
    LEFT JOIN stock_industry si ON si.stock_industry_id = u.stock_industry_id
    LEFT JOIN LATERAL (
        SELECT
            MAX(yoy) FILTER (WHERE rn = 1) AS revenue_yoy,
            MAX(month) FILTER (WHERE rn = 1) AS latest_month,
            MAX(monthly) FILTER (WHERE rn = 1) AS monthly_revenue,
            COALESCE(
                MIN(rn) FILTER (WHERE NOT (yoy > 0 AND ord = first_ord - rn + 1)) - 1,
                COUNT(*)
            ) AS revenue_yoy_streak,
            CASE WHEN COUNT(*) FILTER (WHERE rn <= 3 AND ord = first_ord - rn + 1) = 3
                  AND SUM(last_year) FILTER (WHERE rn <= 3) > 0
              THEN (SUM(monthly) FILTER (WHERE rn <= 3)
                  / SUM(last_year) FILTER (WHERE rn <= 3) - 1) * 100
            END AS revenue_yoy_3m
        FROM (
            SELECT month, yoy, monthly, last_year, ord,
                   ROW_NUMBER() OVER (ORDER BY ord DESC) AS rn,
                   FIRST_VALUE(ord) OVER (ORDER BY ord DESC) AS first_ord
            FROM (
                SELECT "Date" AS month,
                       "ComparedWithLastYearSameMonth" AS yoy,
                       "Monthly" AS monthly,
                       "LastYearThisMonth" AS last_year,
                       ("Date" / 100)::int * 12 + ("Date" % 100)::int AS ord
                FROM "Revenue"
                WHERE ($19::float8 IS NOT NULL OR $20::smallint IS NOT NULL OR $21::float8 IS NOT NULL
                       OR $16::text = 'revenue_yoy' OR $28::bool OR $29::float8 IS NOT NULL)
                  AND "SecurityCode" = u.stock_symbol
                  AND "Date" <= ((u.revenue_ord - 1) / 12) * 100 + (u.revenue_ord - 1) % 12 + 1
                ORDER BY "Date" DESC
                LIMIT 12
            ) recent
        ) ranked
        HAVING MAX(ord) >= u.revenue_ord - 2
    ) rv ON TRUE
    LEFT JOIN revenue_records rr ON rr.stock_symbol = u.stock_symbol AND rr.month = rv.latest_month
    LEFT JOIN LATERAL (
        SELECT fs.earnings_per_share, fs.return_on_equity
        FROM financial_statement fs
        WHERE ($22::float8 IS NOT NULL OR $23::float8 IS NOT NULL)
          AND fs.security_code = u.stock_symbol
          AND fs.quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
          AND fs.year BETWEEN EXTRACT(YEAR FROM u.date)::int - 1 AND EXTRACT(YEAR FROM u.date)::int
          AND CASE fs.quarter
                  WHEN 'Q1' THEN make_date(fs.year::int, 5, 16)
                  WHEN 'Q2' THEN make_date(fs.year::int, 8, 15)
                  WHEN 'Q3' THEN make_date(fs.year::int, 11, 15)
                  ELSE make_date(fs.year::int + 1, 4, 1)
              END <= u.date
          AND (EXTRACT(YEAR FROM u.date)::int * 4 + EXTRACT(QUARTER FROM u.date)::int)
              - (fs.year::int * 4 + SUBSTRING(fs.quarter FROM 2)::int) <= 3
        ORDER BY fs.year DESC, fs.quarter DESC
        LIMIT 1
    ) fs ON TRUE
    LEFT JOIN LATERAL (
        SELECT cur.gross_margin, cur.operating_margin,
               cur.gross_margin > prev.gross_margin
                   AND cur.operating_margin > prev.operating_margin
                   AND cur.net_margin > prev.net_margin AS margins_up
        FROM (
            SELECT i.fiscal_year, i.quarter,
                   i.gross_profit::float8 / NULLIF(i.revenue, 0) * 100 AS gross_margin,
                   i.operating_profit::float8 / NULLIF(i.revenue, 0) * 100 AS operating_margin,
                   i.net_income::float8 / NULLIF(i.revenue, 0) * 100 AS net_margin
            FROM income_statement i
            WHERE ($24::float8 IS NOT NULL OR $25::float8 IS NOT NULL OR $26::bool
                   OR $16::text = 'operating_margin')
              AND i.source = 'yahoo'
              AND i.stock_symbol = u.stock_symbol
              AND i.period_type = 'single'
              AND i.fiscal_year BETWEEN EXTRACT(YEAR FROM u.date)::int - 1 AND EXTRACT(YEAR FROM u.date)::int
              AND CASE i.quarter
                      WHEN 'Q1' THEN make_date(i.fiscal_year, 5, 16)
                      WHEN 'Q2' THEN make_date(i.fiscal_year, 8, 15)
                      WHEN 'Q3' THEN make_date(i.fiscal_year, 11, 15)
                      ELSE make_date(i.fiscal_year + 1, 4, 1)
                  END <= u.date
              AND (EXTRACT(YEAR FROM u.date)::int * 4 + EXTRACT(QUARTER FROM u.date)::int)
                  - (i.fiscal_year * 4 + SUBSTRING(i.quarter FROM 2)::int) <= 3
            ORDER BY i.fiscal_year DESC, i.quarter DESC
            LIMIT 1
        ) cur
        LEFT JOIN LATERAL (
            SELECT p.gross_profit::float8 / NULLIF(p.revenue, 0) * 100 AS gross_margin,
                   p.operating_profit::float8 / NULLIF(p.revenue, 0) * 100 AS operating_margin,
                   p.net_income::float8 / NULLIF(p.revenue, 0) * 100 AS net_margin
            FROM income_statement p
            WHERE $26::bool
              AND p.source = 'yahoo'
              AND p.stock_symbol = u.stock_symbol
              AND p.period_type = 'single'
              AND p.fiscal_year = cur.fiscal_year - 1
              AND p.quarter = cur.quarter
        ) prev ON TRUE
    ) ym ON TRUE
    LEFT JOIN LATERAL (
        SELECT b.total_liabilities::float8 / NULLIF(b.total_assets, 0) * 100 AS debt_ratio
        FROM balance_sheet b
        WHERE $27::float8 IS NOT NULL
          AND b.source = 'yahoo'
          AND b.stock_symbol = u.stock_symbol
          AND b.period_type = 'single'
          AND b.fiscal_year BETWEEN EXTRACT(YEAR FROM u.date)::int - 1 AND EXTRACT(YEAR FROM u.date)::int
          AND CASE b.quarter
                  WHEN 'Q1' THEN make_date(b.fiscal_year, 5, 16)
                  WHEN 'Q2' THEN make_date(b.fiscal_year, 8, 15)
                  WHEN 'Q3' THEN make_date(b.fiscal_year, 11, 15)
                  ELSE make_date(b.fiscal_year + 1, 4, 1)
              END <= u.date
          AND (EXTRACT(YEAR FROM u.date)::int * 4 + EXTRACT(QUARTER FROM u.date)::int)
              - (b.fiscal_year * 4 + SUBSTRING(b.quarter FROM 2)::int) <= 3
        ORDER BY b.fiscal_year DESC, b.quarter DESC
        LIMIT 1
    ) bs ON TRUE
    LEFT JOIN LATERAL (
        SELECT CASE WHEN COUNT(c.free_cash_flow) = 4 THEN SUM(c.free_cash_flow) END AS free_cash_flow_ttm
        FROM (
            SELECT l.fiscal_year, l.fiscal_year * 4 + SUBSTRING(l.quarter FROM 2)::int AS ord
            FROM cash_flow_statement l
            WHERE $30::bool
              AND l.source = 'yahoo'
              AND l.stock_symbol = u.stock_symbol
              AND l.period_type = 'single'
              AND l.fiscal_year BETWEEN EXTRACT(YEAR FROM u.date)::int - 1 AND EXTRACT(YEAR FROM u.date)::int
              AND CASE l.quarter
                      WHEN 'Q1' THEN make_date(l.fiscal_year, 5, 16)
                      WHEN 'Q2' THEN make_date(l.fiscal_year, 8, 15)
                      WHEN 'Q3' THEN make_date(l.fiscal_year, 11, 15)
                      ELSE make_date(l.fiscal_year + 1, 4, 1)
                  END <= u.date
              AND (EXTRACT(YEAR FROM u.date)::int * 4 + EXTRACT(QUARTER FROM u.date)::int)
                  - (l.fiscal_year * 4 + SUBSTRING(l.quarter FROM 2)::int) <= 3
            ORDER BY l.fiscal_year DESC, l.quarter DESC
            LIMIT 1
        ) latest
        JOIN cash_flow_statement c
          ON c.source = 'yahoo'
         AND c.stock_symbol = u.stock_symbol
         AND c.period_type = 'single'
         AND c.fiscal_year BETWEEN latest.fiscal_year - 1 AND latest.fiscal_year
         AND c.fiscal_year * 4 + SUBSTRING(c.quarter FROM 2)::int BETWEEN latest.ord - 3 AND latest.ord
    ) cf ON TRUE
    LEFT JOIN LATERAL (
        SELECT CASE WHEN q."PriceEarningRatio" > 0 THEN q."PriceEarningRatio"::float8 END AS pe,
               CASE WHEN q."price-to-book_ratio" > 0 THEN q."price-to-book_ratio"::float8 END AS pb
        FROM "DailyQuotes" q
        WHERE ($31::float8 IS NOT NULL OR $32::float8 IS NOT NULL OR $16::text IN ('pe_ratio', 'pb_ratio'))
          AND q.stock_symbol = u.stock_symbol
          AND q."Date" = u.date
    ) qv ON TRUE
    LEFT JOIN holder_rebal hr ON hr.date = u.date
    LEFT JOIN holder_distribution hc ON hc.stock_symbol = u.stock_symbol AND hc."date" = hr.cur_week
    LEFT JOIN holder_distribution hb ON hb.stock_symbol = u.stock_symbol AND hb."date" = hr.base_week"#,
            $chip_join,
            r#"
    WHERE ($6::text IS NULL OR si.name = $6)
      AND ($18::text[] IS NULL OR si.name IS NULL OR NOT si.name = ANY($18))
      AND ($7::smallint IS NULL OR u.rs_score >= $7)
      AND ($8::smallint IS NULL OR u.rs_score <= $8)
      AND (($9::text IS NULL AND $10::text IS NULL) OR EXISTS (
          SELECT 1 FROM daily_technical_signal t
          WHERE t.stock_symbol = u.stock_symbol
            AND t.date BETWEEN u.window_start AND u.date
            AND ($9::text IS NULL OR t.indicator = $9)
            AND ($10::text IS NULL OR t.kind = $10)
      ))
      AND ($19::float8 IS NULL OR rv.revenue_yoy >= $19)
      AND ($20::smallint IS NULL OR rv.revenue_yoy_streak >= $20)
      AND ($21::float8 IS NULL OR rv.revenue_yoy_3m >= $21)
      AND ($22::float8 IS NULL OR fs.earnings_per_share >= $22)
      AND ($23::float8 IS NULL OR fs.return_on_equity >= $23)
      AND ($24::float8 IS NULL OR ym.gross_margin >= $24)
      AND ($25::float8 IS NULL OR ym.operating_margin >= $25)
      AND (NOT $26::bool OR ym.margins_up IS TRUE)
      AND ($27::float8 IS NULL OR bs.debt_ratio <= $27)
      AND (NOT $28::bool OR rr.is_record IS TRUE)
      AND ($29::float8 IS NULL OR rv.monthly_revenue >= $29)
      AND (NOT $30::bool OR cf.free_cash_flow_ttm > 0)
      AND ($31::float8 IS NULL OR qv.pe <= $31)
      AND ($32::float8 IS NULL OR qv.pb <= $32)
      AND ($35::float8 IS NULL OR (hc.major_percent - hb.major_percent)::float8 >= $35)"#,
            $chip_filter,
            "\n)"
        )
    };
}

pub(super) use filtered_cte;
