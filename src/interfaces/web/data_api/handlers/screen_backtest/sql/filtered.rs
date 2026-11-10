//! 回測 SQL 的 `filtered` CTE：符合流動性以外條件的股票（營收、財報、估值、大戶、籌碼）。

/// 展開 `filtered` CTE（含結尾的右括號）；`$chip_columns`／`$chip_join`／`$chip_filter` 為籌碼版的欄位、
/// JOIN 與條件（無籌碼版傳空字串與 `NULL` 欄位）。各段子查詢的規則見 [`super::BACKTEST_SQL`] 的說明。
macro_rules! filtered_cte {
    ($chip_columns:literal, $chip_join:literal, $chip_filter:literal) => {
        concat!(
            r#"filtered AS (
    SELECT u.date, u.liquidity_start, u.stock_symbol, u.rs_score, u.rs_rank, u.ret, u.stock_industry_id,
           rv.revenue_yoy::float8 AS revenue_yoy, ym.operating_margin, qv.pe, qv.pb, qv.bias240,
           CASE WHEN qv.close > 0 AND qv.close * mi.shares + mi.net_debt > 0
               THEN mi.ebit_ttm / (qv.close * mi.shares + mi.net_debt) END AS earnings_yield,
           CASE WHEN mi.capital > 0 THEN mi.ebit_ttm / mi.capital END AS return_on_capital,
           -- 成長率超過 100%（多為低基期轉盈）時 PEG 失真，不計算。
           CASE WHEN qv.pe > 0 AND eg.growth > 0 AND eg.growth <= 100 THEN qv.pe / eg.growth END AS peg,
           rk.volatility, pp.pb_percentile, rdr.rd_ratio,
           -- 葛拉漢乘積：本益比 × 股價淨值比（`qv` 已把 0 以下轉成 NULL，虧損股沒有資料）。
           qv.pe * qv.pb AS pe_pb_product,"#,
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
            -- 營收加速：從最新月往回，年增率高於前一個月且月份相連的月數。
            COALESCE(
                MIN(rn) FILTER (WHERE NOT COALESCE(
                    yoy > prev_yoy AND prev_ord = ord - 1 AND ord = first_ord - rn + 1, FALSE)) - 1,
                COUNT(*)
            ) AS revenue_accel_months,
            CASE WHEN COUNT(*) FILTER (WHERE rn <= 3 AND ord = first_ord - rn + 1) = 3
                  AND SUM(last_year) FILTER (WHERE rn <= 3) > 0
              THEN (SUM(monthly) FILTER (WHERE rn <= 3)
                  / SUM(last_year) FILTER (WHERE rn <= 3) - 1) * 100
            END AS revenue_yoy_3m
        FROM (
            SELECT month, yoy, monthly, last_year, ord,
                   ROW_NUMBER() OVER (ORDER BY ord DESC) AS rn,
                   FIRST_VALUE(ord) OVER (ORDER BY ord DESC) AS first_ord,
                   LEAD(yoy) OVER (ORDER BY ord DESC) AS prev_yoy,
                   LEAD(ord) OVER (ORDER BY ord DESC) AS prev_ord
            FROM (
                SELECT "Date" AS month,
                       "ComparedWithLastYearSameMonth" AS yoy,
                       "Monthly" AS monthly,
                       "LastYearThisMonth" AS last_year,
                       ("Date" / 100)::int * 12 + ("Date" % 100)::int AS ord
                FROM "Revenue"
                WHERE ($19::float8 IS NOT NULL OR $20::smallint IS NOT NULL OR $21::float8 IS NOT NULL
                       OR $16::text = 'revenue_yoy' OR $28::bool OR $29::float8 IS NOT NULL
                       OR $45::smallint IS NOT NULL)
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
        SELECT fs.year, fs.quarter, fs.earnings_per_share, fs.return_on_equity
        FROM financial_statement fs
        WHERE ($22::float8 IS NOT NULL OR $23::float8 IS NOT NULL OR $37::float8 IS NOT NULL)
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
        -- CANSLIM 的 C：上面那一季對去年同一季的 EPS 成長率；去年同季 ≤ 0 為 NULL。
        SELECT CASE WHEN p.earnings_per_share > 0
            THEN ((fs.earnings_per_share - p.earnings_per_share) / p.earnings_per_share * 100)::float8
        END AS eps_yoy
        FROM financial_statement p
        WHERE $37::float8 IS NOT NULL
          AND p.security_code = u.stock_symbol
          AND p.year = fs.year - 1
          AND p.quarter = fs.quarter
    ) fy ON TRUE
    LEFT JOIN LATERAL (
        -- CANSLIM 的 A：換股當天已公布（隔年 4/1 起）的最近 4 個年度 EPS，3 個年增率取最低者；年度須連續、
        -- 基期都 > 0、最新年度落後換股年 1～2 年，否則為 NULL。
        SELECT CASE WHEN COUNT(*) = 4 AND MAX(year) - MIN(year) = 3
                     AND COUNT(*) FILTER (WHERE prev_eps > 0) = 3
                     AND EXTRACT(YEAR FROM u.date)::int - MAX(year) BETWEEN 1 AND 2
                THEN MIN((eps - prev_eps) / NULLIF(prev_eps, 0) * 100)::float8
            END AS annual_growth
        FROM (
            SELECT year, eps, LEAD(eps) OVER (ORDER BY year DESC) AS prev_eps
            FROM (
                SELECT a.year, a.earnings_per_share AS eps
                FROM financial_statement a
                WHERE $38::float8 IS NOT NULL
                  AND a.security_code = u.stock_symbol
                  AND a.quarter = ''
                  AND make_date(a.year::int + 1, 4, 1) <= u.date
                ORDER BY a.year DESC
                LIMIT 4
            ) recent
        ) annual
    ) ae ON TRUE
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
               CASE WHEN q."price-to-book_ratio" > 0 THEN q."price-to-book_ratio"::float8 END AS pb,
               q."ClosingPrice"::float8 AS close,
               -- 年線乖離率（%）：收盤相對 240 日均線；年線為 0（上市未滿一年）時沒有資料。
               CASE WHEN q."MovingAverage240" > 0
                   THEN (q."ClosingPrice" / q."MovingAverage240" - 1)::float8 * 100 END AS bias240
        FROM "DailyQuotes" q
        WHERE ($31::float8 IS NOT NULL OR $32::float8 IS NOT NULL OR $50::float8 IS NOT NULL
               OR $53::float8 IS NOT NULL OR $54::float8 IS NOT NULL OR $58::float8 IS NOT NULL
               OR $16::text IN ('pe_ratio', 'pb_ratio', 'magic_formula', 'peg', 'bias240', 'pe_pb_product'))
          AND q.stock_symbol = u.stock_symbol
          AND q."Date" = u.date
    ) qv ON TRUE
    LEFT JOIN LATERAL (
        -- Minervini 趨勢模板：換股當天收盤後的日線（與收盤買進同一時點）MA20 > MA60 > MA120 > MA240、收盤站上 MA240、收盤不低於 52 週（240 個交易日）最高價的 75%、且至少比 52 週最低價高 30%；
        -- 年線或 52 週最低價為 0（資料不足）時沒有資料、不會通過。只在設 `$47` 時才查。
        SELECT q."MovingAverage20" > q."MovingAverage60"
               AND q."MovingAverage60" > q."MovingAverage120"
               AND q."MovingAverage120" > q."MovingAverage240"
               AND q."ClosingPrice" > q."MovingAverage240"
               AND q."ClosingPrice" >= q.maximum_price_in_year * 0.75
               AND q."ClosingPrice" >= q.minimum_price_in_year * 1.3 AS passed
        FROM "DailyQuotes" q
        WHERE $47::bool
          AND q.stock_symbol = u.stock_symbol
          AND q."Date" = u.date
          AND q."MovingAverage240" > 0
          AND q.minimum_price_in_year > 0
    ) tt ON TRUE
    LEFT JOIN LATERAL (
        -- 融資餘額變化（%）：融資資料收盤後才公布，換股日只用前一個交易日（含）以前 10 天內最近一筆，
        -- 對同一檔往前第 20 筆；基期為 0（不能融資或沒有資料）時沒有資料。只在設 `$49` 時才查。
        SELECT (cur.margin_balance - base.margin_balance)::float8 * 100
                   / NULLIF(base.margin_balance, 0)::float8 AS margin_change
        FROM (
            SELECT m."date", m.margin_balance
            FROM chip_daily m
            WHERE $49::float8 IS NOT NULL
              AND m.stock_symbol = u.stock_symbol
              AND m."date" < u.date
              AND m."date" >= u.date - 10
            ORDER BY m."date" DESC
            LIMIT 1
        ) cur
        CROSS JOIN LATERAL (
            SELECT b.margin_balance
            FROM chip_daily b
            WHERE b.stock_symbol = u.stock_symbol AND b."date" < cur."date"
            ORDER BY b."date" DESC
            OFFSET 19
            LIMIT 1
        ) base
    ) mg ON TRUE
    LEFT JOIN stock_rights sr ON sr.stock_symbol = u.stock_symbol
    LEFT JOIN LATERAL (
        -- 現金股利（dividend_fill_event）只用換股當天（含）以前的除息。殖利率：最近一次除息往回 350 天的現金股利
        -- ÷ 換股日收盤（最近一次除息須在 400 天內），每筆現金股利再除以除息日到換股日（皆含）之間配股除權的股數
        -- 倍數 1 + 股票股利 ÷ 10（`stock_rights`），換股日收盤已是除權後的價格；填息率與平均填息天數：除息日在近 5 年內、觀察期 120 個交易日
        -- 在換股日以前已滿的事件；連續配息年數：以除息日年度計，最近一次除息的年度不早於換股年前一年。
        SELECT
            CASE WHEN $41::float8 IS NOT NULL AND latest.ex_date > u.date - 400 THEN (
                SELECT SUM(y.cash_dividend::float8 / COALESCE((
                    SELECT EXP(SUM(LN(1 + r.stock_dividend / 10)))
                    FROM unnest(sr.ex_rights, sr.stock_dividends) AS r(ex_rights, stock_dividend)
                    WHERE r.ex_rights BETWEEN y.ex_date AND u.date
                ), 1)) FROM dividend_fill_event y
                WHERE y.stock_symbol = u.stock_symbol
                  AND y.ex_date > latest.ex_date - 350 AND y.ex_date <= latest.ex_date
            ) * 100 / NULLIF((
                SELECT q."ClosingPrice" FROM "DailyQuotes" q
                WHERE q.stock_symbol = u.stock_symbol AND q."Date" = u.date
            ), 0)::float8 END AS dividend_yield,
            fill.fill_rate, fill.avg_fill_days, streak.dividend_years
        FROM (
            SELECT MAX(l.ex_date) AS ex_date FROM dividend_fill_event l
            WHERE l.stock_symbol = u.stock_symbol AND l.ex_date <= u.date
        ) latest
        CROSS JOIN LATERAL (
            SELECT COUNT(*) FILTER (WHERE f.days_to_fill IS NOT NULL)::float8 * 100 / NULLIF(COUNT(*), 0) AS fill_rate,
                   AVG(f.days_to_fill)::float8 AS avg_fill_days
            FROM dividend_fill_event f
            WHERE ($42::float8 IS NOT NULL OR $43::float8 IS NOT NULL)
              AND f.stock_symbol = u.stock_symbol
              AND f.ex_date > u.date - 1826
              AND f.mature_date <= u.date
        ) fill
        CROSS JOIN LATERAL (
            SELECT (COUNT(*) FILTER (WHERE yr = top - rn + 1
                    AND top >= EXTRACT(YEAR FROM u.date)::int - 1))::int AS dividend_years
            FROM (
                SELECT yr, ROW_NUMBER() OVER (ORDER BY yr DESC) AS rn, MAX(yr) OVER () AS top
                FROM (
                    SELECT DISTINCT EXTRACT(YEAR FROM s.ex_date)::int AS yr
                    FROM dividend_fill_event s
                    WHERE $44::int IS NOT NULL AND s.stock_symbol = u.stock_symbol AND s.ex_date <= u.date
                ) years
            ) ranked
        ) streak
        WHERE $41::float8 IS NOT NULL OR $42::float8 IS NOT NULL OR $43::float8 IS NOT NULL
           OR $44::int IS NOT NULL
    ) dv ON TRUE
    -- 股利成長年數：換股年本身還沒過完，以前一年（已完整）的成長年數判斷。
    LEFT JOIN dividend_growth dg
           ON dg.stock_symbol = u.stock_symbol AND dg.yr = EXTRACT(YEAR FROM u.date)::int - 1
    -- 換股日已過法定期限的最近一季（Q1 5/16、Q2 8/15、Q3 11/15、Q4 隔年 4/1；所有股票同一季），`ord` = 年 × 4 + 季；
    -- 神奇公式與 PEG 都對上這一季。
    CROSS JOIN LATERAL (
        SELECT EXTRACT(YEAR FROM u.date)::int * 4 + CASE
                   WHEN u.date >= make_date(EXTRACT(YEAR FROM u.date)::int, 11, 15) THEN 3
                   WHEN u.date >= make_date(EXTRACT(YEAR FROM u.date)::int, 8, 15) THEN 2
                   WHEN u.date >= make_date(EXTRACT(YEAR FROM u.date)::int, 5, 16) THEN 1
                   WHEN u.date >= make_date(EXTRACT(YEAR FROM u.date)::int, 4, 1) THEN 0
                   ELSE -1
               END AS ord
    ) pq
    LEFT JOIN magic_inputs mi ON mi.stock_symbol = u.stock_symbol AND mi.ord = pq.ord
    LEFT JOIN eps_growth eg ON eg.stock_symbol = u.stock_symbol AND eg.ord = pq.ord
    LEFT JOIN share_counts sc ON sc.stock_symbol = u.stock_symbol AND sc.ord = pq.ord
    -- `pq` 是 Q4 以前的季時，最新年度報告是前一年；尚未過 4/1（`pq` 為前一年 Q3）時是前兩年。
    LEFT JOIN profit_streak ps ON ps.stock_symbol = u.stock_symbol AND ps.yr = pq.ord / 4 - 1
    LEFT JOIN LATERAL (
        -- 低波動：換股當天收盤後的風險指標（近 240 筆日線，與收盤買進同一時點）；只在設 `$51`／`$52` 或依低波動
        -- 排序時才查（主鍵查詢）。
        SELECT r.volatility_percent::float8 AS volatility, r.beta::float8 AS beta
        FROM daily_risk_metric r
        WHERE ($51::float8 IS NOT NULL OR $52::float8 IS NOT NULL OR $16::text = 'low_volatility')
          AND r.date = u.date
          AND r.stock_symbol = u.stock_symbol
    ) rk ON TRUE
    LEFT JOIN LATERAL (
        -- 便宜價：換股當天收盤後的股價淨值比近 5 年百分位（與收盤買進同一時點）；只在設 `$56` 或依淨值比分位
        -- 排序時才查（主鍵查詢）。
        SELECT p.pb_percentile::float8 AS pb_percentile
        FROM daily_pb_percentile p
        WHERE ($56::float8 IS NOT NULL OR $16::text = 'pb_percentile')
          AND p.date = u.date
          AND p.stock_symbol = u.stock_symbol
    ) pp ON TRUE
    LEFT JOIN LATERAL (
        -- 研發費用率（%）：換股日已公布最近一季往回四季（季別須相連、最新一季在 3 季內）的研發費用合計 ÷ 營收合計；
        -- 公布時點同三率（`ym`）。只在設 `$57` 或依研發費用率排序時才查。
        SELECT SUM(r.rd_expenses)::float8 * 100 / NULLIF(SUM(r.revenue), 0)::float8 AS rd_ratio
        FROM (
            SELECT i.rd_expenses, i.revenue, i.fiscal_year * 4 + SUBSTRING(i.quarter FROM 2)::int AS ord
            FROM income_statement i
            WHERE ($57::float8 IS NOT NULL OR $16::text = 'rd_ratio')
              AND i.source = 'yahoo'
              AND i.stock_symbol = u.stock_symbol
              AND i.period_type = 'single'
              AND i.fiscal_year BETWEEN EXTRACT(YEAR FROM u.date)::int - 2 AND EXTRACT(YEAR FROM u.date)::int
              AND CASE i.quarter
                          WHEN 'Q1' THEN make_date(i.fiscal_year, 5, 16)
                          WHEN 'Q2' THEN make_date(i.fiscal_year, 8, 15)
                          WHEN 'Q3' THEN make_date(i.fiscal_year, 11, 15)
                          ELSE make_date(i.fiscal_year + 1, 4, 1)
                      END <= u.date
            ORDER BY i.fiscal_year DESC, i.quarter DESC
            LIMIT 4
        ) r
        HAVING COUNT(r.rd_expenses) = 4 AND COUNT(r.revenue) = 4 AND MAX(r.ord) - MIN(r.ord) = 3
           AND (EXTRACT(YEAR FROM u.date)::int * 4 + EXTRACT(QUARTER FROM u.date)::int) - MAX(r.ord) <= 3
    ) rdr ON TRUE
    LEFT JOIN LATERAL (
        -- 流動比率（倍）：換股日已公布最新一季（3 季內）Yahoo 資產負債表的流動資產 ÷ 流動負債；公布時點同三率，
        -- 流動負債不大於 0 時沒有資料。只在設 `$59` 時才查。
        SELECT CASE WHEN b.current_liabilities > 0
                   THEN b.current_assets::float8 / b.current_liabilities::float8 END AS current_ratio
        FROM balance_sheet b
        WHERE $59::float8 IS NOT NULL
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
    ) bsr ON TRUE
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
      AND ($35::float8 IS NULL OR COALESCE((hc.major_percent - hb.major_percent)::float8 >= $35, FALSE))
      AND ($48::float8 IS NULL OR COALESCE((hc.percent_400 - hb.percent_400)::float8 >= $48, FALSE))
      AND ($37::float8 IS NULL OR fy.eps_yoy >= $37)
      AND ($38::float8 IS NULL OR ae.annual_growth >= $38)
      AND ($39::int IS NULL OR EXISTS (
          SELECT 1 FROM daily_technical_signal n
          WHERE n.stock_symbol = u.stock_symbol
            AND n.indicator = '52_week'
            AND n.kind = 'new_high'
            AND n.date BETWEEN u.date - $39::int AND u.date
      ))
      -- Piotroski F 分數：換股當天已公布（隔年 4/1 起）的最近一個評分年度，落後換股年 1～2 年才算數。
      AND ($40::smallint IS NULL OR (
          SELECT fh.f_score FROM financial_f_score_history fh
          WHERE fh.stock_symbol = u.stock_symbol
            AND make_date(fh.fiscal_year + 1, 4, 1) <= u.date
            AND fh.fiscal_year >= EXTRACT(YEAR FROM u.date)::int - 2
          ORDER BY fh.fiscal_year DESC
          LIMIT 1
      ) >= $40)
      AND ($41::float8 IS NULL OR dv.dividend_yield >= $41)
      AND ($42::float8 IS NULL OR dv.fill_rate >= $42)
      AND ($43::float8 IS NULL OR dv.avg_fill_days <= $43)
      AND ($44::int IS NULL OR dv.dividend_years >= $44)
      AND ($45::smallint IS NULL OR rv.revenue_accel_months >= $45)
      AND ($46::int IS NULL OR dg.growth_years >= $46)
      AND (NOT $47::bool OR tt.passed IS TRUE)
      AND ($49::float8 IS NULL OR mg.margin_change <= $49)
      -- 包 COALESCE：嚴格比較會讓 eps_growth 的 LEFT JOIN 降成 INNER JOIN、改變連接順序。
      AND ($50::float8 IS NULL OR COALESCE(qv.pe > 0 AND eg.growth > 0 AND eg.growth <= 100 AND qv.pe / eg.growth <= $50, FALSE))
      AND ($51::float8 IS NULL OR COALESCE(rk.volatility <= $51, FALSE))
      AND ($52::float8 IS NULL OR COALESCE(rk.beta <= $52, FALSE))
      AND ($53::float8 IS NULL OR COALESCE(qv.bias240 <= $53, FALSE))
      -- 市值（億元）= 收盤 × 千股 × 1000 ÷ 1 億；沒有該季股本時改用目前發行股數（股）。
      AND ($54::float8 IS NULL OR COALESCE(qv.close * COALESCE(sc.shares, (
          SELECT NULLIF(st.issued_share, 0)::float8 / 1000 FROM stocks st WHERE st.stock_symbol = u.stock_symbol
      )) >= $54 * 100000, FALSE))
      AND ($55::int IS NULL OR COALESCE(ps.profit_years >= $55, FALSE))
      AND ($56::float8 IS NULL OR COALESCE(pp.pb_percentile <= $56, FALSE))
      AND ($57::float8 IS NULL OR COALESCE(rdr.rd_ratio >= $57, FALSE))
      AND ($58::float8 IS NULL OR COALESCE(qv.pe * qv.pb <= $58, FALSE))
      AND ($59::float8 IS NULL OR COALESCE(bsr.current_ratio >= $59, FALSE))"#,
            $chip_filter,
            "\n)"
        )
    };
}

pub(super) use filtered_cte;
