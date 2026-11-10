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
/// `$28` 是否只選創新高；`$29` 排除的產業 id（`NULL` 為不排除）。
/// 本益比、股價淨值比取查詢日（含）之前 10 天內最新一筆日線的官方值（0 代表虧損或沒有資料，轉成
/// NULL）；`$30`／`$31` 為上限（`NULL` 為不限）。
/// 千張大戶持股比例變化由 handler 以 [`MAJOR_CHANGES_SQL`](super::signal_sql::MAJOR_CHANGES_SQL) 先算好，以 `$32` 代號、`$33` 變化（百分點）兩個平行
/// 陣列左接，`$34` 為下限（`NULL` 為不限；沒有資料的股票不會通過）。
/// 近期區間突破由 handler 以 [`RANGE_BREAKOUTS_SQL`](super::signal_sql::RANGE_BREAKOUTS_SQL) 先算好，以 `$35` 代號、`$36` 突破日、`$37` 說明三個平行
/// 陣列左接；`$38` 為 true 時只留有突破的股票。
/// CANSLIM 的 C／A／N：單季 EPS 年增率比較最新一季與去年同一季（去年同季 ≤ 0 為 NULL），新鮮度同 EPS，`$39` 為
/// 下限；年度 EPS 成長取最近 4 個年度（`quarter = ''`）算出的 3 個年增率中最低者，年度須連續、基期皆 > 0，
/// 最新年度落後查詢年 1～2 年才算新鮮，`$40` 為下限；`$41` 有值時才查 52 週新高訊號（一年內最近一次），
/// 只留 `$41` 個日曆天內創新高的股票。
/// 存股條件讀 `dividend_fill_event`：近 5 年（除息日在查詢日前 5 年內、120 個交易日觀察期已滿）填息率下限 `$42`、
/// 已填息者平均填息交易日數上限 `$43`、連續配息年數下限 `$44`；三個欄位每檔都會回傳。
/// 營收加速：從最新月往回年增率逐月提高（高於前一個月、月份相連）的月數，新鮮度同月營收，`$45` 為下限。
/// 股利成長年數：只看查詢年以前的完整年度（年度現金股利合計），從查詢年前一年往回每年不低於前一年（年度相連）
/// 的年數，`$46` 為下限；每檔都會回傳。
/// Minervini 趨勢模板：查詢日前 10 天內最新一筆日線MA20 > MA60 > MA120 > MA240、收盤站上 MA240、收盤不低於 52 週（240 個交易日）最高價的 75%、且至少比 52 週最低價高 30%；年線或 52 週最低價為 0 時為
/// `NULL`。每檔都會回傳，`$47` 為 true 時只留符合的股票。
/// 400 張以上大戶持股比例變化與千張變化一起由 handler 先算好，以 `$48` 平行陣列左接，`$49` 為下限；
/// 融資餘額變化：查詢日前 10 天內最新一筆一筆對往前第 20 筆（約一個月）的變化率（%），負值代表融資減少；基期融資餘額為 0 （不能融資或沒有資料）的不會通過，只在設 `$50`（上限）時才查。
/// 神奇公式（Greenblatt）：Yahoo 最新一季資產負債表與截至該季的近四季單季營業利益（EBIT，四季須相連），新鮮度同
/// 三率（兩季內）。盈餘殖利率 = EBIT ÷（`pv` 收盤 × 股數 + 淨負債），資本報酬率 = EBIT ÷（max(淨營運資金, 0) +
/// 不動產廠房設備），算法同回測 `magic_inputs`；`magic_formula_rank` 是篩選後兩者都有資料的股票中兩個名次相加
/// （越小越好），`magic_formula_ranked_count` 是排名的母體檔數，供 `sort_by=magic_formula`；另附算式的分子分母
/// （近四季營業利益、投入資本、企業價值，千元）。
/// PEG：Yahoo 最近 8 季單季 EPS（須相連）的近四季合計對前四季合計成長率（%，新鮮度同三率、前四季合計須 > 0），
/// `peg_ratio` = `pv` 本益比 ÷ 成長率（虧損或成長率不在 0～100% 時為 NULL），`$51` 為上限；供 `sort_by=peg`。
/// 低波動：查詢日前 10 天內最新一筆 `daily_risk_metric`（近 240 筆日線的年化波動度與 beta），`$52`／`$53` 為上限；
/// 供 `sort_by=volatility`／`beta`。
/// 年線乖離率：`pv` 同一筆日線收盤相對 240 日均線（%，年線為 0 時為 NULL），`$54` 為上限；供 `sort_by=bias240`。
/// 市值（億元）：`pv` 收盤 × 目前發行股數（`stocks.issued_share`，為 0 時為 NULL），`$55` 為下限。連續獲利年數：已公布
/// （隔年 4/1 起）的年度 EPS（`quarter = ''`）從最新年度往回連續 > 0（年度相連）的年數，最新年度落後查詢年超過 2 年
/// 為 0，`$56` 為下限；兩者每檔都會回傳。
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
        r.revenue_accel_raw,
        r.revenue_yoy_3m_raw,
        rh.revenue_record_raw,
        f.year AS financial_year,
        f.quarter AS financial_quarter,
        f.earnings_per_share AS eps_raw,
        f.return_on_equity AS roe_raw,
        fy.eps_yoy_raw,
        ae.annual_eps_year,
        ae.annual_eps_growth_raw,
        nh.new_high_date,
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
        q.avg_trade_value,
        pv.ratio_date,
        pv.pe_ratio,
        pv.pb_ratio,
        pv.trend_template,
        mg.margin_change_percent,
        pv.close_price,
        pv.bias240_percent,
        mf.magic_year,
        mf.magic_quarter,
        mf.ebit_ttm,
        mf.magic_capital,
        mf.magic_net_debt,
        mf.magic_shares,
        eg.eps_ord,
        eg.ttm_eps,
        eg.prev_ttm_eps,
        rk.volatility_percent,
        rk.beta,
        dv.fill_rate_percent,
        dv.avg_fill_days,
        dv.dividend_years,
        dv.dividend_growth_years,
        ROUND(pv.close_price * NULLIF(s.issued_share, 0) / 100000000.0, 2) AS market_cap,
        pf.profit_years
    FROM stocks s
    LEFT JOIN LATERAL (
        -- 最近 12 個月營收：最新一月的年增率、從最新月往回連續年增（> 0 且月份相連）
        -- 的月數、從最新月往回年增率逐月提高（高於前一個月且月份相連）的月數（營收加速），
        -- 以及近 3 個月（須相連）合計營收對去年同期的年增率。
        SELECT
            MAX(month) FILTER (WHERE rn = 1) AS revenue_raw_month,
            MAX(yoy) FILTER (WHERE rn = 1) AS revenue_raw_yoy,
            MAX(monthly) FILTER (WHERE rn = 1) AS revenue_raw_monthly,
            COALESCE(
                MIN(rn) FILTER (WHERE NOT (yoy > 0 AND ord = first_ord - rn + 1)) - 1,
                COUNT(*)
            )::smallint AS revenue_yoy_streak_raw,
            COALESCE(
                MIN(rn) FILTER (WHERE NOT COALESCE(
                    yoy > prev_yoy AND prev_ord = ord - 1 AND ord = first_ord - rn + 1, FALSE)) - 1,
                COUNT(*)
            )::smallint AS revenue_accel_raw,
            CASE WHEN COUNT(*) FILTER (WHERE rn <= 3 AND ord = first_ord - rn + 1) = 3
                  AND SUM(last_year) FILTER (WHERE rn <= 3) > 0
              THEN ROUND((SUM(monthly) FILTER (WHERE rn <= 3)
                  / SUM(last_year) FILTER (WHERE rn <= 3) - 1) * 100, 2)
            END AS revenue_yoy_3m_raw
        FROM (
            SELECT month, yoy, monthly, last_year, ord,
                ROW_NUMBER() OVER (ORDER BY month DESC) AS rn,
                FIRST_VALUE(ord) OVER (ORDER BY month DESC) AS first_ord,
                LEAD(yoy) OVER (ORDER BY month DESC) AS prev_yoy,
                LEAD(ord) OVER (ORDER BY month DESC) AS prev_ord
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
        -- 最新一季單季 EPS 對去年同一季的成長率；去年同季 EPS ≤ 0 時成長率沒有意義，為 NULL。
        SELECT CASE WHEN p.earnings_per_share > 0
            THEN ROUND((f.earnings_per_share - p.earnings_per_share) / p.earnings_per_share * 100, 2)
        END AS eps_yoy_raw
        FROM financial_statement p
        WHERE p.security_code = s.stock_symbol
          AND p.year = f.year - 1
          AND p.quarter = f.quarter
    ) fy ON TRUE
    LEFT JOIN LATERAL (
        -- 最近 4 個年度 EPS 的 3 個年增率取最低者：年度須連續、3 個基期都 > 0，否則為 NULL。
        SELECT MAX(year)::int AS annual_eps_year,
            CASE WHEN COUNT(*) = 4 AND MAX(year) - MIN(year) = 3
                  AND COUNT(*) FILTER (WHERE prev_eps > 0) = 3
              THEN ROUND(MIN((eps - prev_eps) / NULLIF(prev_eps, 0) * 100), 2)
            END AS annual_eps_growth_raw
        FROM (
            SELECT year, eps, LEAD(eps) OVER (ORDER BY year DESC) AS prev_eps
            FROM (
                SELECT year, earnings_per_share AS eps
                FROM financial_statement
                WHERE security_code = s.stock_symbol
                  AND quarter = ''
                ORDER BY year DESC
                LIMIT 4
            ) recent
        ) annual
    ) ae ON TRUE
    LEFT JOIN LATERAL (
        -- 查詢日（含）以前一年內最近一次 52 週新高；只有設 `new_high_days`（$41）時才查。
        SELECT MAX(date) AS new_high_date
        FROM daily_technical_signal
        WHERE $41::int IS NOT NULL
          AND stock_symbol = s.stock_symbol
          AND indicator = '52_week'
          AND kind = 'new_high'
          AND date BETWEEN $3::date - 365 AND $3::date
    ) nh ON TRUE
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
        -- 查詢日（含）之前 10 天內最新一筆日線的官方本益比、股價淨值比；0（虧損或沒有資料）轉成 NULL。
        SELECT "Date" AS ratio_date,
            "ClosingPrice" AS close_price,
            CASE WHEN "MovingAverage240" > 0
                THEN ROUND(("ClosingPrice" / "MovingAverage240" - 1) * 100, 2) END AS bias240_percent,
            CASE WHEN "PriceEarningRatio" > 0 THEN "PriceEarningRatio" END AS pe_ratio,
            CASE WHEN "price-to-book_ratio" > 0 THEN "price-to-book_ratio" END AS pb_ratio,
            -- Minervini 趨勢模板（同一筆日線）；年線或 52 週最低價為 0（資料不足）時為 NULL。
            CASE WHEN "MovingAverage240" > 0 AND minimum_price_in_year > 0 THEN
                "MovingAverage20" > "MovingAverage60"
                AND "MovingAverage60" > "MovingAverage120"
                AND "MovingAverage120" > "MovingAverage240"
                AND "ClosingPrice" > "MovingAverage240"
                AND "ClosingPrice" >= maximum_price_in_year * 0.75
                AND "ClosingPrice" >= minimum_price_in_year * 1.3
            END AS trend_template
        FROM "DailyQuotes"
        WHERE stock_symbol = s.stock_symbol
          AND "Date" BETWEEN $3::date - 10 AND $3::date
        ORDER BY "Date" DESC
        LIMIT 1
    ) pv ON TRUE
    LEFT JOIN LATERAL (
        -- 融資餘額：查詢日前 10 天內最新一筆對同一檔往前第 20 筆的變化率（%）；基期為 0 時為 NULL。
        SELECT ROUND((cur.margin_balance - base.margin_balance) * 100.0
                     / NULLIF(base.margin_balance, 0), 2) AS margin_change_percent
        FROM (
            SELECT m."date", m.margin_balance
            FROM chip_daily m
            WHERE $50::numeric IS NOT NULL
              AND m.stock_symbol = s.stock_symbol
              AND m."date" BETWEEN $3::date - 10 AND $3::date
            ORDER BY m."date" DESC
            LIMIT 1
        ) cur
        CROSS JOIN LATERAL (
            SELECT b.margin_balance
            FROM chip_daily b
            WHERE b.stock_symbol = s.stock_symbol AND b."date" < cur."date"
            ORDER BY b."date" DESC
            OFFSET 19
            LIMIT 1
        ) base
    ) mg ON TRUE
    LEFT JOIN LATERAL (
        -- 神奇公式：Yahoo 最新一季資產負債表與截至該季的近四季單季營業利益（四季須相連）。資本 = max(流動資產 − 現金
        -- −（流動負債 − 短期借款 − 應付短期票券 − 一年內到期長期負債）, 0) + 不動產廠房設備；淨負債 = 上述有息負債 +
        -- 長期負債 − 現金；股數用目前發行股數（沒有時以股本 ÷ 10 推算）。流動資產或流動負債為 0（金融業）時沒有資料。
        SELECT b.fiscal_year AS magic_year,
            SUBSTRING(b.quarter FROM 2)::smallint AS magic_quarter,
            e.ebit_ttm,
            GREATEST(
                (b.current_assets - COALESCE(b.cash_and_equivalents, 0))
                - (b.current_liabilities - COALESCE(b.short_term_debt, 0)
                   - COALESCE(b.short_term_bills_payable, 0)
                   - COALESCE(b.current_portion_of_long_term_liabilities, 0)),
                0) + b.property_plant_equipment AS magic_capital,
            COALESCE(b.short_term_debt, 0) + COALESCE(b.short_term_bills_payable, 0)
                + COALESCE(b.current_portion_of_long_term_liabilities, 0) + COALESCE(b.long_term_liabilities, 0)
                - COALESCE(b.cash_and_equivalents, 0) AS magic_net_debt,
            COALESCE(NULLIF(s.issued_share, 0) / 1000.0, b.share_capital / 10.0) AS magic_shares
        FROM (
            SELECT fiscal_year, quarter, cash_and_equivalents, current_assets, current_liabilities,
                short_term_debt, short_term_bills_payable, current_portion_of_long_term_liabilities,
                long_term_liabilities, property_plant_equipment, share_capital
            FROM balance_sheet
            WHERE source = 'yahoo'
              AND stock_symbol = s.stock_symbol
              AND period_type = 'single'
            ORDER BY fiscal_year DESC, quarter DESC
            LIMIT 1
        ) b
        CROSS JOIN LATERAL (
            SELECT SUM(i.operating_profit) AS ebit_ttm, COUNT(i.operating_profit) AS quarters
            FROM income_statement i
            WHERE i.source = 'yahoo'
              AND i.stock_symbol = s.stock_symbol
              AND i.period_type = 'single'
              AND i.fiscal_year BETWEEN b.fiscal_year - 1 AND b.fiscal_year
              AND i.fiscal_year * 4 + SUBSTRING(i.quarter FROM 2)::int
                  BETWEEN b.fiscal_year * 4 + SUBSTRING(b.quarter FROM 2)::int - 3
                      AND b.fiscal_year * 4 + SUBSTRING(b.quarter FROM 2)::int
        ) e
        WHERE e.quarters = 4
          AND b.current_assets > 0
          AND b.current_liabilities > 0
          AND b.property_plant_equipment IS NOT NULL
          AND b.share_capital > 0
    ) mf ON TRUE
    LEFT JOIN LATERAL (
        -- PEG：Yahoo 最近 8 季單季 EPS（須相連），近四季合計與前四季合計。
        SELECT MAX(ord) AS eps_ord,
            SUM(eps) FILTER (WHERE rn <= 4) AS ttm_eps,
            SUM(eps) FILTER (WHERE rn > 4) AS prev_ttm_eps
        FROM (
            SELECT fiscal_year * 4 + SUBSTRING(quarter FROM 2)::int AS ord, eps,
                ROW_NUMBER() OVER (ORDER BY fiscal_year DESC, quarter DESC) AS rn
            FROM income_statement
            WHERE source = 'yahoo'
              AND stock_symbol = s.stock_symbol
              AND period_type = 'single'
            ORDER BY fiscal_year DESC, quarter DESC
            LIMIT 8
        ) recent
        HAVING COUNT(eps) = 8 AND MAX(ord) - MIN(ord) = 7
    ) eg ON TRUE
    LEFT JOIN LATERAL (
        -- 低波動：查詢日前 10 天內最新一筆風險指標。
        SELECT volatility_percent, beta
        FROM daily_risk_metric
        WHERE stock_symbol = s.stock_symbol
          AND date BETWEEN $3::date - 10 AND $3::date
        ORDER BY date DESC
        LIMIT 1
    ) rk ON TRUE
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
    LEFT JOIN LATERAL (
        -- 近 5 年填息（dividend_fill_event）：除息日在查詢日前 5 年內、觀察期 120 個交易日已滿的現金除息；
        -- 連續配息年數以除息日年度計，最近一次除息的年度早於查詢年前一年時為 0。
        SELECT fill.fill_rate_percent, fill.avg_fill_days, streak.dividend_years, growth.dividend_growth_years
        FROM (
            SELECT ROUND(COUNT(*) FILTER (WHERE f.days_to_fill IS NOT NULL) * 100.0
                       / NULLIF(COUNT(*), 0), 1) AS fill_rate_percent,
                   ROUND(AVG(f.days_to_fill), 1) AS avg_fill_days
            FROM dividend_fill_event f
            WHERE f.stock_symbol = s.stock_symbol
              AND f.ex_date > $3::date - 1826
              AND f.mature_date <= $3::date
        ) fill
        CROSS JOIN (
            SELECT (COUNT(*) FILTER (WHERE yr = top - rn + 1
                    AND top >= EXTRACT(YEAR FROM $3::date)::int - 1))::int AS dividend_years
            FROM (
                SELECT yr, ROW_NUMBER() OVER (ORDER BY yr DESC) AS rn, MAX(yr) OVER () AS top
                FROM (
                    SELECT DISTINCT EXTRACT(YEAR FROM d.ex_date)::int AS yr
                    FROM dividend_fill_event d
                    WHERE d.stock_symbol = s.stock_symbol AND d.ex_date <= $3::date
                ) years
            ) ranked
        ) streak
        CROSS JOIN (
            SELECT CASE WHEN MAX(yr) >= EXTRACT(YEAR FROM $3::date)::int - 1
                        THEN (MIN(rn) FILTER (WHERE NOT grew) - 1)::int ELSE 0 END AS dividend_growth_years
            FROM (
                SELECT yr, ROW_NUMBER() OVER (ORDER BY yr DESC) AS rn,
                       COALESCE(LEAD(yr) OVER (ORDER BY yr DESC) = yr - 1
                                AND cash >= LEAD(cash) OVER (ORDER BY yr DESC), FALSE) AS grew
                FROM (
                    SELECT EXTRACT(YEAR FROM g.ex_date)::int AS yr, SUM(g.cash_dividend) AS cash
                    FROM dividend_fill_event g
                    WHERE g.stock_symbol = s.stock_symbol
                      AND g.ex_date < make_date(EXTRACT(YEAR FROM $3::date)::int, 1, 1)
                    GROUP BY 1
                ) years
            ) ranked
        ) growth
    ) dv ON TRUE
    LEFT JOIN LATERAL (
        -- 連續獲利年數：已公布（隔年 4/1 起）的年度 EPS 從最新年度往回連續 > 0、年度相連的年數。
        SELECT CASE WHEN MAX(yr) >= EXTRACT(YEAR FROM $3::date)::int - 2
                    THEN COALESCE(MIN(rn) FILTER (WHERE NOT (eps > 0 AND yr = top - rn + 1)) - 1, COUNT(*))
                    ELSE 0 END::int AS profit_years
        FROM (
            SELECT a.year::int AS yr, a.earnings_per_share AS eps,
                   ROW_NUMBER() OVER (ORDER BY a.year DESC) AS rn, MAX(a.year::int) OVER () AS top
            FROM financial_statement a
            WHERE a.security_code = s.stock_symbol
              AND a.quarter = ''
              AND a.year > 0
              AND make_date(a.year::int + 1, 4, 1) <= $3::date
        ) annual
    ) pf ON TRUE
    LEFT JOIN financial_health h ON h.stock_symbol = s.stock_symbol
    WHERE (($1::int = 0 AND s.stock_exchange_market_id IN (2, 4))
        OR ($1::int <> 0 AND s.stock_exchange_market_id = $1))
      AND ($2::int IS NULL OR s.stock_industry_id = $2)
      AND ($29::int[] IS NULL OR NOT s.stock_industry_id = ANY($29))
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
          THEN revenue_accel_raw END AS revenue_acceleration_months,
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
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN eps_yoy_raw END AS eps_yoy_percent,
        CASE WHEN EXTRACT(YEAR FROM $3::date)::int - annual_eps_year BETWEEN 1 AND 2
          THEN annual_eps_growth_raw END AS annual_eps_growth_percent,
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
          THEN margins_up_raw END AS margins_yoy_up,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (magic_year * 4 + magic_quarter)) BETWEEN 0 AND 2
          AND close_price > 0 AND close_price * magic_shares + magic_net_debt > 0
          THEN ROUND(ebit_ttm * 100 / (close_price * magic_shares + magic_net_debt), 2)
        END AS earnings_yield_percent,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (magic_year * 4 + magic_quarter)) BETWEEN 0 AND 2
          AND magic_capital > 0
          THEN ROUND(ebit_ttm * 100 / magic_capital, 2)
        END AS return_on_capital_percent,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (magic_year * 4 + magic_quarter)) BETWEEN 0 AND 2
          THEN ebit_ttm::numeric END AS operating_income_ttm,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (magic_year * 4 + magic_quarter)) BETWEEN 0 AND 2
          THEN magic_capital::numeric END AS invested_capital,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (magic_year * 4 + magic_quarter)) BETWEEN 0 AND 2
          AND close_price > 0
          THEN ROUND(close_price * magic_shares + magic_net_debt) END AS enterprise_value,
        CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int) - eps_ord) BETWEEN 0 AND 2
          AND prev_ttm_eps > 0
          THEN ROUND((ttm_eps / prev_ttm_eps - 1) * 100, 2)
        END AS eps_ttm_growth_percent
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
    avg_trade_value,
    pe_ratio, pb_ratio, ratio_date,
    brk.breakout_date, brk.breakout_description,
    eps_yoy_percent, annual_eps_growth_percent, annual_eps_year, new_high_date,
    fill_rate_percent, avg_fill_days, dividend_years, revenue_acceleration_months, dividend_growth_years,
    trend_template, margin_change_percent,
    earnings_yield_percent, return_on_capital_percent, eps_ttm_growth_percent,
    CASE WHEN pe_ratio > 0 AND eps_ttm_growth_percent > 0 AND eps_ttm_growth_percent <= 100
        THEN ROUND(pe_ratio / eps_ttm_growth_percent, 2) END AS peg_ratio,
    -- 視窗函式在 WHERE 之後計算：只在篩選後、兩個比率都有資料的股票之間排名。
    CASE WHEN earnings_yield_percent IS NOT NULL AND return_on_capital_percent IS NOT NULL THEN
        RANK() OVER (PARTITION BY earnings_yield_percent IS NULL OR return_on_capital_percent IS NULL
                     ORDER BY earnings_yield_percent DESC)
        + RANK() OVER (PARTITION BY earnings_yield_percent IS NULL OR return_on_capital_percent IS NULL
                       ORDER BY return_on_capital_percent DESC)
    END AS magic_formula_rank,
    CASE WHEN earnings_yield_percent IS NOT NULL AND return_on_capital_percent IS NOT NULL THEN
        COUNT(*) OVER (PARTITION BY earnings_yield_percent IS NULL OR return_on_capital_percent IS NULL)
    END AS magic_formula_ranked_count,
    operating_income_ttm, invested_capital, enterprise_value, volatility_percent, beta, bias240_percent,
    market_cap, profit_years
FROM normalized
LEFT JOIN UNNEST($16::text[], $17::smallint[]) AS rs(symbol, rs_score)
       ON rs.symbol = normalized.stock_symbol
LEFT JOIN UNNEST($19::text[], $20::smallint[], $21::smallint[])
       AS chip(symbol, foreign_streak, trust_streak)
       ON chip.symbol = normalized.stock_symbol
LEFT JOIN UNNEST($32::text[], $33::numeric[], $48::numeric[]) AS major(symbol, major_change, holder_400_change)
       ON major.symbol = normalized.stock_symbol
LEFT JOIN UNNEST($35::text[], $36::date[], $37::text[])
       AS brk(symbol, breakout_date, breakout_description)
       ON brk.symbol = normalized.stock_symbol
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
  AND ($30::numeric IS NULL OR pe_ratio <= $30)
  AND ($31::numeric IS NULL OR pb_ratio <= $31)
  AND ($34::numeric IS NULL OR major.major_change >= $34)
  AND (NOT $38::bool OR brk.symbol IS NOT NULL)
  AND ($39::numeric IS NULL OR eps_yoy_percent >= $39)
  AND ($40::numeric IS NULL OR annual_eps_growth_percent >= $40)
  AND ($41::int IS NULL OR new_high_date >= $3::date - $41)
  AND ($42::numeric IS NULL OR fill_rate_percent >= $42)
  AND ($43::numeric IS NULL OR avg_fill_days <= $43)
  AND ($44::int IS NULL OR dividend_years >= $44)
  AND ($45::smallint IS NULL OR revenue_acceleration_months >= $45)
  AND ($46::int IS NULL OR dividend_growth_years >= $46)
  AND (NOT $47::bool OR trend_template IS TRUE)
  AND ($49::numeric IS NULL OR major.holder_400_change >= $49)
  AND ($50::numeric IS NULL OR margin_change_percent <= $50)
  AND ($51::numeric IS NULL OR (pe_ratio > 0 AND eps_ttm_growth_percent > 0 AND eps_ttm_growth_percent <= 100
      AND ROUND(pe_ratio / eps_ttm_growth_percent, 2) <= $51))
  AND ($52::numeric IS NULL OR volatility_percent <= $52)
  AND ($53::numeric IS NULL OR beta <= $53)
  AND ($54::numeric IS NULL OR bias240_percent <= $54)
  AND ($55::numeric IS NULL OR market_cap >= $55)
  AND ($56::int IS NULL OR profit_years >= $56)
"#;
