//! 條件選股的參數化 SQL 主體。

/// 條件選股的參數化 SQL 主體。
///
/// 月營收取每股最近 12 個月計算動能（連續年增月數、近 3 月累計年增率），
/// 其餘 `LATERAL ... LIMIT 1` 會使用各表以股票代號為前導欄位的索引，找出「每股
/// 各自最新」資料；`financial_health` 每檔只有一列，直接以主鍵左接。
/// `normalized` 僅將過期指標轉成 SQL NULL；最外層條件利用 NULL 比較不成立的
/// 語意，自然排除以該過期指標篩選的股票。相對強度分數由 handler 以全體計算後，
/// 以 `$16` 代號、`$17` 分數兩個平行陣列傳入左接（沒有分數為 NULL）；外資、投信連續
/// 買賣超天數同樣由 handler 算好，以 `$19` 代號、`$20` 外資、`$21` 投信三個平行陣列左接。
/// 平均成交金額取查詢日（含）之前 45 天內最近 20 筆日線（`DailyQuotes` 以代號、日期為前導的唯一
/// 索引），`$25` 為門檻（元，`NULL` 為不限）；`$24` 是外層附加的 `LIMIT`。
/// 營業利益率、稅後淨利率與三率三升取 Yahoo 最新一季單季損益表（`income_statement_history_idx`），
/// 三率三升比較去年同一季；新鮮度同財務體質（兩季內）。`$26` 營業利益率下限、`$27` 是否只選三率三升。
/// 營收創歷史新高：最新一月營收高於之前所有月份（之前至少 12 個月才判斷），新鮮度同月營收；
/// `$28` 是否只選創新高。
pub(super) const SCREEN_STOCKS_SQL: &str = r#"
WITH latest AS (
    SELECT
        s.stock_symbol,
        s."Name" AS name,
        s.stock_exchange_market_id AS market_id,
        s.stock_industry_id AS industry_id,
        r.revenue_raw_month,
        r.revenue_raw_yoy,
        r.revenue_raw_monthly,
        r.revenue_yoy_streak_raw,
        r.revenue_yoy_3m_raw,
        rh.revenue_record_raw,
        f.year AS financial_year,
        f.quarter AS financial_quarter,
        f.earnings_per_share AS eps_raw,
        f.return_on_equity AS roe_raw,
        e.date AS valuation_raw_date,
        e.percentage AS valuation_raw_percentage,
        e.closing_price,
        e.cheap,
        e.fair,
        e.expensive,
        y.date AS yield_raw_date,
        y.yield AS yield_raw,
        h.f_score_year,
        h.f_score AS f_score_raw,
        h.f_score_evaluated AS f_score_evaluated_raw,
        h.quarter_year AS health_quarter_year,
        h.quarter AS health_quarter,
        h.gross_margin_percent AS gross_margin_raw,
        h.debt_ratio_percent AS debt_ratio_raw,
        h.free_cash_flow_ttm AS free_cash_flow_raw,
        m.margin_year,
        m.margin_quarter,
        m.operating_margin_raw,
        m.net_margin_raw,
        m.margins_up_raw,
        q.avg_trade_value
    FROM stocks s
    LEFT JOIN LATERAL (
        -- 最近 12 個月營收：最新一月的年增率、從最新月往回連續年增（> 0 且月份相連）
        -- 的月數，以及近 3 個月（須相連）合計營收對去年同期的年增率。
        SELECT
            MAX(month) FILTER (WHERE rn = 1) AS revenue_raw_month,
            MAX(yoy) FILTER (WHERE rn = 1) AS revenue_raw_yoy,
            MAX(monthly) FILTER (WHERE rn = 1) AS revenue_raw_monthly,
            COALESCE(
                MIN(rn) FILTER (WHERE NOT (yoy > 0 AND ord = first_ord - rn + 1)) - 1,
                COUNT(*)
            )::smallint AS revenue_yoy_streak_raw,
            CASE WHEN COUNT(*) FILTER (WHERE rn <= 3 AND ord = first_ord - rn + 1) = 3
                  AND SUM(last_year) FILTER (WHERE rn <= 3) > 0
              THEN ROUND((SUM(monthly) FILTER (WHERE rn <= 3)
                  / SUM(last_year) FILTER (WHERE rn <= 3) - 1) * 100, 2)
            END AS revenue_yoy_3m_raw
        FROM (
            SELECT month, yoy, monthly, last_year, ord,
                ROW_NUMBER() OVER (ORDER BY month DESC) AS rn,
                FIRST_VALUE(ord) OVER (ORDER BY month DESC) AS first_ord
            FROM (
                SELECT "Date" AS month,
                    "ComparedWithLastYearSameMonth" AS yoy,
                    "Monthly" AS monthly,
                    "LastYearThisMonth" AS last_year,
                    ("Date" / 100) * 12 + "Date" % 100 AS ord
                FROM "Revenue"
                WHERE "SecurityCode" = s.stock_symbol
                ORDER BY "Date" DESC
                LIMIT 12
            ) recent
        ) ranked
    ) r ON TRUE
    LEFT JOIN LATERAL (
        -- 最新一月之前所有月份的最高營收；之前不滿 12 個月不判斷（NULL）。
        SELECT CASE WHEN COUNT(*) >= 12 THEN r.revenue_raw_monthly > MAX("Monthly") END
                   AS revenue_record_raw
        FROM "Revenue"
        WHERE "SecurityCode" = s.stock_symbol
          AND "Date" < r.revenue_raw_month
    ) rh ON TRUE
    LEFT JOIN LATERAL (
        SELECT year, quarter, earnings_per_share, return_on_equity
        FROM financial_statement
        WHERE security_code = s.stock_symbol
          AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
        ORDER BY year DESC,
            CASE quarter
                WHEN 'Q4' THEN 4 WHEN 'Q3' THEN 3
                WHEN 'Q2' THEN 2 WHEN 'Q1' THEN 1
            END DESC
        LIMIT 1
    ) f ON TRUE
    LEFT JOIN LATERAL (
        SELECT date, percentage, closing_price, cheap, fair, expensive
        FROM estimate
        WHERE security_code = s.stock_symbol
        ORDER BY date DESC
        LIMIT 1
    ) e ON TRUE
    LEFT JOIN LATERAL (
        SELECT date, yield
        FROM yield_rank
        WHERE security_code = s.stock_symbol
        ORDER BY date DESC
        LIMIT 1
    ) y ON TRUE
    LEFT JOIN LATERAL (
        -- 近 20 筆日線的平均成交金額（元）；停牌超過 45 天沒有日線時為 NULL。
        SELECT AVG(recent_quotes."TradeValue") AS avg_trade_value
        FROM (
            SELECT "TradeValue"
            FROM "DailyQuotes"
            WHERE stock_symbol = s.stock_symbol
              AND "Date" BETWEEN $3::date - 45 AND $3::date
            ORDER BY "Date" DESC
            LIMIT 20
        ) recent_quotes
    ) q ON TRUE
    LEFT JOIN LATERAL (
        -- Yahoo 最新一季單季損益表：營業利益率、稅後淨利率，以及毛利率、營業利益率、稅後淨利率
        -- 是否都高於去年同一季（三率三升；去年同季沒有資料或營收為 0 時為 NULL）。
        SELECT cur.fiscal_year AS margin_year,
            SUBSTRING(cur.quarter FROM 2)::smallint AS margin_quarter,
            ROUND(cur.operating_profit::numeric / NULLIF(cur.revenue, 0) * 100, 2) AS operating_margin_raw,
            ROUND(cur.net_income::numeric / NULLIF(cur.revenue, 0) * 100, 2) AS net_margin_raw,
            cur.gross_profit::numeric / NULLIF(cur.revenue, 0)
                    > prev.gross_profit::numeric / NULLIF(prev.revenue, 0)
                AND cur.operating_profit::numeric / NULLIF(cur.revenue, 0)
                    > prev.operating_profit::numeric / NULLIF(prev.revenue, 0)
                AND cur.net_income::numeric / NULLIF(cur.revenue, 0)
                    > prev.net_income::numeric / NULLIF(prev.revenue, 0) AS margins_up_raw
        FROM (
            SELECT fiscal_year, quarter, revenue, gross_profit, operating_profit, net_income
            FROM income_statement
            WHERE source = 'yahoo'
              AND stock_symbol = s.stock_symbol
              AND period_type = 'single'
            ORDER BY fiscal_year DESC, quarter DESC
            LIMIT 1
        ) cur
        LEFT JOIN income_statement prev
               ON prev.source = 'yahoo'
              AND prev.stock_symbol = s.stock_symbol
              AND prev.period_type = 'single'
              AND prev.fiscal_year = cur.fiscal_year - 1
              AND prev.quarter = cur.quarter
    ) m ON TRUE
    LEFT JOIN financial_health h ON h.stock_symbol = s.stock_symbol
    WHERE (($1::int = 0 AND s.stock_exchange_market_id IN (2, 4))
        OR ($1::int <> 0 AND s.stock_exchange_market_id = $1))
      AND ($2::int IS NULL OR s.stock_industry_id = $2)
), normalized AS (
    SELECT
        latest.*,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_raw_yoy END AS revenue_yoy_percent,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_yoy_streak_raw END AS revenue_yoy_streak,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_yoy_3m_raw END AS revenue_yoy_3m_percent,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_raw_monthly END AS monthly_revenue,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_record_raw END AS revenue_record,
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN eps_raw END AS earnings_per_share,
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN roe_raw END AS return_on_equity,
        CASE WHEN valuation_raw_date BETWEEN $3::date - 30 AND $3::date
          THEN valuation_raw_percentage END AS valuation_percentage,
        CASE WHEN valuation_raw_date BETWEEN $3::date - 30 AND $3::date THEN
            CASE
                WHEN closing_price <= cheap THEN 'undervalued'
                WHEN closing_price <= fair THEN 'fair_valued'
                WHEN closing_price <= expensive THEN 'overvalued'
                ELSE 'highly_overvalued'
            END
        END AS valuation_band,
        CASE WHEN yield_raw_date BETWEEN $3::date - 30 AND $3::date
          THEN yield_raw END AS dividend_yield_percent,
        CASE WHEN EXTRACT(YEAR FROM $3::date)::int - f_score_year BETWEEN 1 AND 2
          THEN f_score_raw END AS f_score,
        CASE WHEN EXTRACT(YEAR FROM $3::date)::int - f_score_year BETWEEN 1 AND 2
          THEN f_score_evaluated_raw END AS f_score_evaluated,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (health_quarter_year * 4 + health_quarter)) BETWEEN 0 AND 2
          THEN gross_margin_raw END AS gross_margin_percent,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (health_quarter_year * 4 + health_quarter)) BETWEEN 0 AND 2
          THEN debt_ratio_raw END AS debt_ratio_percent,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (health_quarter_year * 4 + health_quarter)) BETWEEN 0 AND 2
          THEN free_cash_flow_raw END AS free_cash_flow_ttm,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (margin_year * 4 + margin_quarter)) BETWEEN 0 AND 2
          THEN operating_margin_raw END AS operating_margin_percent,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (margin_year * 4 + margin_quarter)) BETWEEN 0 AND 2
          THEN net_margin_raw END AS net_margin_percent,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (margin_year * 4 + margin_quarter)) BETWEEN 0 AND 2
          THEN margins_up_raw END AS margins_yoy_up
    FROM latest
)
SELECT
    stock_symbol, name, market_id, industry_id,
    revenue_yoy_percent, revenue_yoy_streak, revenue_yoy_3m_percent, monthly_revenue, revenue_record,
    earnings_per_share, return_on_equity, dividend_yield_percent, valuation_band, valuation_percentage,
    revenue_raw_month AS revenue_month,
    financial_year, financial_quarter,
    valuation_raw_date AS valuation_date,
    yield_raw_date AS yield_date,
    f_score, f_score_evaluated, f_score_year,
    gross_margin_percent, debt_ratio_percent, free_cash_flow_ttm,
    health_quarter_year, health_quarter,
    operating_margin_percent, net_margin_percent, margins_yoy_up, margin_year, margin_quarter,
    rs.rs_score,
    chip.foreign_streak,
    chip.trust_streak,
    avg_trade_value
FROM normalized
LEFT JOIN UNNEST($16::text[], $17::smallint[]) AS rs(symbol, rs_score)
       ON rs.symbol = normalized.stock_symbol
LEFT JOIN UNNEST($19::text[], $20::smallint[], $21::smallint[])
       AS chip(symbol, foreign_streak, trust_streak)
       ON chip.symbol = normalized.stock_symbol
WHERE ($4::numeric IS NULL OR revenue_yoy_percent >= $4)
  AND ($5::numeric IS NULL OR earnings_per_share >= $5)
  AND ($6::numeric IS NULL OR return_on_equity >= $6)
  AND ($7::numeric IS NULL OR dividend_yield_percent >= $7)
  AND ($8::text IS NULL OR valuation_band = $8)
  AND ($9::smallint IS NULL OR f_score >= $9)
  AND ($10::numeric IS NULL OR gross_margin_percent >= $10)
  AND ($11::numeric IS NULL OR debt_ratio_percent <= $11)
  AND (NOT $12::bool OR free_cash_flow_ttm > 0)
  AND ($13::smallint IS NULL OR revenue_yoy_streak >= $13)
  AND ($14::numeric IS NULL OR revenue_yoy_3m_percent >= $14)
  AND ($15::numeric IS NULL OR monthly_revenue >= $15)
  AND ($18::smallint IS NULL OR rs.rs_score >= $18)
  AND ($22::smallint IS NULL OR chip.foreign_streak >= $22)
  AND ($23::smallint IS NULL OR chip.trust_streak >= $23)
  AND ($25::numeric IS NULL OR avg_trade_value >= $25)
  AND ($26::numeric IS NULL OR operating_margin_percent >= $26)
  AND (NOT $27::bool OR margins_yoy_up IS TRUE)
  AND (NOT $28::bool OR revenue_record IS TRUE)
"#;
