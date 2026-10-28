//! 條件選股回測的 SQL：最新相對強度日與回測主體（有、無籌碼條件兩版）。

use super::super::signal_history::chip_streak_ctes;
use crate::infra::database::repository::relative_strength::rs_rank_order;

#[cfg(doc)]
use super::{ETF_BENCHMARK, portfolio::select_holdings};

/// `$1`（含）之前最近一個有相對強度的交易日；`$1` 為 `NULL` 時取最新。
pub(super) const LATEST_RS_DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_relative_strength WHERE $1::date IS NULL OR date <= $1
"#;

/// 集保股權分散最早的資料週；設 `min_major_change` 時用來檢查起日。
pub(super) const FIRST_HOLDER_WEEK_SQL: &str = r#"
SELECT MIN("date") FROM holder_distribution
"#;

/// 回測 SQL 的主體。`$1`／`$2` 起訖、`$3` 市場 id（0 為上市櫃全部）、`$4` 持有天數、`$5` 每期候選檔數、
/// `$6` 產業、`$7`／`$8` 相對強度上下限、`$9`／`$10` 訊號來源與種類、`$11` 大盤濾網均線天數
/// （`NULL` 為不濾）、`$12` 續抱的相對強度門檻（`NULL` 為不設）、`$13` 市場廣度門檻（%，`NULL` 為不濾）、
/// `$14` 換股日偏移（起日之後先跳過幾個交易日）、`$15` 流動性門檻（元，`NULL` 為不限）、`$16`
/// 候選股排序方式（見 [`super::RANK_BY`]）、`$17` 持股加權方式（見 [`super::portfolio::Weighting`]）、`$18`
/// 排除的產業名稱（`NULL` 為不排除）、`$19`～`$23` 基本面門檻（月營收年增率、連續年增月數、近 3 月
/// 年增率、每股盈餘、股東權益報酬率，`NULL` 為不篩）、`$24`～`$27` 財報三率與負債比（毛利率、營業
/// 利益率下限、是否只選三率三升、負債比上限）、`$28` 是否只選最新月營收創歷史新高、`$29` 最新月營收
/// 下限（千元，`NULL` 為不篩）、`$30` 是否只選近四季自由現金流為正、`$31`／`$32` 換股當天本益比與股價
/// 淨值比上限（倍，`NULL` 為不篩）、`$33` 大盤估值濾網（市場本益比中位數上限，`NULL` 為不濾）、`$34` 相對
/// 估值濾網（市場本益比中位數在近 5 年的百分位上限，`NULL` 為不濾）、`$35` 千張大戶持股比例變化下限
/// （百分點，`NULL` 為不篩）、`$36` 變化的回看週數；籌碼版另有 `$37`／`$38`。
///
/// - `days`：有相對強度的交易日（往前多讀 400 個日曆天，讓第一期也能回看訊號；往後多讀 400 個日曆天，
///   計算最後幾期是否在迄日（`$2`）前完成出場）。
/// - `rebal`：起日之後先跳過 `$14` 個交易日、再每隔 `$4` 個交易日的換股日；`window_start` 是
///   含當天往前第 `$4` 個交易日，訊號條件看 `window_start`～換股日；`liquidity_start` 是含當天往前
///   第 20 個交易日（之前不足 20 個交易日時取最早一天），流動性看 `liquidity_start`～換股日的平均
///   成交金額；`exit_date` 為換股日收盤買進後持有 `$4` 個交易日的出場日（期末收盤賣出）。
/// - `bench`：同一天同市場全部母體股票的平均報酬；`exit_date > $2` 或不足持有天數時視為未完成
///   （`pending = true`），報酬欄位與 0050 皆為 `NULL`，不偷看迄日之後的行情；`samples` 為 0 代表
///   還沒滿持有天數，不列入完整期數，只有最後一個換股日例外：以 `pending` 標記列出，當作策略目前的
///   持股（`elapsed_days` 為換股日之後到迄日經過的交易日數）。
/// - `rebal` 另算 `revenue_ord`：換股日當天已公布的最新營收月份（年 × 12 + 月）。每月營收最晚次月 10 日
///   公布，次月 11 日起才使用，即換股日往前 10 天所在月份的前一個月。
/// - `filtered` 的 `rv`：已公布的最近 12 個月營收，算最新一月營收與年增率、連續年增月數與近 3 月年增率（算法同
///   `/stocks/screen`）；最新一筆早於 `revenue_ord` 超過 2 個月視為沒有資料。`fs`：已公布的最近一季財報，
///   以法定期限的隔天起才使用（Q1 5/16、Q2 8/15、Q3 11/15、Q4 隔年 4/1），距換股日超過 3 季不用。兩者都
///   只在有設對應門檻時才查。
/// - `holder_weeks`／`holder_rebal`：只在設 `$35` 時才算。`holder_weeks` 是 `holder_distribution` 的全部資料週
///   依日期編號；`holder_rebal` 為每個換股日取日期早於換股日的最近一週（集保週五的資料週六才公布，換股
///   當天還看不到），以及它往前第 `$36` 週。最近一週距換股日超過 14 天時該期沒有大戶資料。`filtered` 以
///   兩週的 `holder_distribution` 千張比例相減，任一週沒有該股資料就不會通過。
/// - `revenue_records`：每檔每個月的營收是否高於之前所有月份（之前至少 12 個月），以視窗函式一次算完全表，
///   `filtered` 以 `rv` 的最新一月（已公布）對上；只在 `$28` 時才算（逐期逐檔掃歷史要多 5 秒以上）。
/// - `filtered` 的 `ym`：已公布的最近一季 Yahoo 單季損益表（公布日規則同 `fs`）的毛利率、營業利益率，
///   以及毛利率、營業利益率、稅後淨利率是否都高於去年同一季（三率三升）；`bs`：同樣規則的最近一季
///   資產負債表負債比；`cf`：同樣規則的最近一季 Yahoo 單季現金流量表，連同之前 3 季（須四季都有自由
///   現金流）的合計。同樣只在有設門檻時才查。
/// - `filtered` 的 `qv`：換股當天日線的官方本益比與股價淨值比（收盤後公布，與收盤買進同一時點），只在
///   有設 `$31`／`$32` 或依兩者排序時才查；值為 0 代表虧損或沒有資料，轉成 NULL（有設門檻時不會通過、
///   排序時排最後）。
/// - `filtered`／`valued`：符合流動性以外條件的股票；有設流動性門檻、依成交金額排序或依成交金額
///   加權時，才對這些股票逐檔查 `liquidity_start`～換股日的平均成交金額（`trade_value`）。
/// - `picks`／`candidates`：再通過流動性門檻的股票依 `$16` 排序（`rs_score` 為相對強度，其餘為
///   成交金額或法人連續買超天數，本益比與股價淨值比由低到高，同值再依相對強度分數與名次 `rs_rank`）的前 `$5` 檔與各自的報酬、產業編號、平均成交
///   金額；實際持股由 [`select_holdings`] 依上一期持股、`keep_rank` 與 `max_per_industry` 決定。
/// - `holdable`：相對強度不低於 `$12` 的母體股票，供上一期持股判斷能否續抱（不看其他選股條件）；
///   依成交金額加權時同樣逐檔查平均成交金額。
/// - `taiex`：加權指數收盤是否不低於含當天往前 `$11` 個交易日的平均；低於時 `market_ok` 為
///   false，這期空手。
/// - `breadth`：同市場站上 60 日均線的股票比例（`daily_stock_price_stats`）是否不低於 `$13`%；
///   低於時同樣空手。換股日沒有統計列視為通過（與加權指數缺資料相同）。
/// - `valuation_level`：同市場（`$3`）換股日的本益比中位數（`daily_market_valuation`）是否不高於 `$33`，
///   且在換股日之前近 5 年（不含當天）的每日中位數中，低於當天值的比例未達 `$34`%；任一不符時同樣空手。
///   百分位只用換股日以前的資料，不會偷看之後的行情；換股日沒有估值列、或之前不足 250 個交易日時視為通過。
/// - `etf_return`：同一天買進 [`ETF_BENCHMARK`] 持有同樣天數的還原報酬，作為實際買得到的大盤基準。
///
/// 成交價模型（`$entry`／`$exit`／`$ret`／`$price_joins`／`$etf_ret`／`$etf_joins`）：收盤版
/// `entry_date` 就是換股日，報酬取換股日的同天期還原報酬；次日開盤版 `entry_date` 為次一交易日、
/// `exit_date` 再晚一天，報酬取 `entry_date` 起的同天期收盤報酬，再乘上出場日與進場日各自的
/// 開盤價／收盤價比（同一天的還原因子相消，不需另查還原價），把收盤到收盤換成開盤到開盤；
/// 開盤或收盤價缺漏時比值取 1（沿用收盤）。選股條件仍只看換股日當天與之前的資料。
macro_rules! backtest_sql {
    ($with:literal, $chip_join:literal, $chip_filter:literal, $chip_columns:literal,
     $entry:expr, $exit:expr, $ret:expr, $price_joins:expr, $etf_ret:expr,
     $etf_joins:expr) => {
        concat!(
            $with,
            r#"
days AS (
    SELECT date FROM daily_relative_strength
    WHERE date BETWEEN $1::date - 400 AND ($2::date + 400)
    GROUP BY date
),
windowed AS (
    SELECT date,
           LAG(date, $4::int - 1, date) OVER (ORDER BY date) AS window_start,
           COALESCE(LAG(date, 19) OVER (ORDER BY date), MIN(date) OVER (ORDER BY date))
               AS liquidity_start,
           "#,
            $entry,
            r#" AS entry_date,
           "#,
            $exit,
            r#" AS exit_date
    FROM days
),
rebal AS (
    SELECT date, window_start, liquidity_start, entry_date, exit_date,
           EXTRACT(YEAR FROM date - 10)::int * 12 + EXTRACT(MONTH FROM date - 10)::int - 1
               AS revenue_ord
    FROM (
        SELECT date, window_start, liquidity_start, entry_date, exit_date, ROW_NUMBER() OVER (ORDER BY date) - 1 AS i
        FROM windowed
        WHERE date >= $1 AND date <= $2
    ) AS numbered
    WHERE i >= $14::int AND (i - $14::int) % $4::int = 0
),
holder_weeks AS (
    SELECT "date", ROW_NUMBER() OVER (ORDER BY "date") AS wk
    FROM (SELECT DISTINCT "date" FROM holder_distribution WHERE $35::float8 IS NOT NULL) AS weeks
),
holder_rebal AS (
    SELECT rb.date, cw."date" AS cur_week, bw."date" AS base_week
    FROM rebal rb
    CROSS JOIN LATERAL (
        SELECT "date", wk FROM holder_weeks WHERE "date" < rb.date ORDER BY "date" DESC LIMIT 1
    ) cw
    JOIN holder_weeks bw ON bw.wk = cw.wk - $36::int
    WHERE cw."date" >= rb.date - 14
),
revenue_records AS (
    SELECT "SecurityCode" AS stock_symbol, "Date" AS month,
           COUNT(*) OVER prior >= 12 AND "Monthly" > MAX("Monthly") OVER prior AS is_record
    FROM "Revenue"
    WHERE $28::bool
    WINDOW prior AS (
        PARTITION BY "SecurityCode" ORDER BY "Date" ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
    )
),
universe AS (
    SELECT rb.date, rb.window_start, rb.liquidity_start, rb.revenue_ord, rb.entry_date, rb.exit_date, rs.stock_symbol, rs.rs_score,
           rs.rank AS rs_rank,
           s.stock_industry_id,
           CASE
               WHEN rb.exit_date IS NOT NULL AND rb.exit_date <= $2 THEN
                   "#,
            $ret,
            r#"
               ELSE NULL
           END AS ret
    FROM rebal rb
    JOIN daily_relative_strength rs ON rs.date = rb.date
    JOIN stocks s ON s.stock_symbol = rs.stock_symbol
    LEFT JOIN daily_forward_return f ON f.date = rb.entry_date AND f.stock_symbol = rs.stock_symbol"#,
            $price_joins,
            r#"
    WHERE ($3 = 0 OR s.stock_exchange_market_id = $3)
),
bench AS (
    SELECT date, entry_date, exit_date, AVG(ret)::float8 AS avg_ret, COUNT(ret) AS samples
    FROM universe
    GROUP BY date, entry_date, exit_date
),
filtered AS (
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
            r#"
),
-- 成交金額同時用於排序與權重；明確物化，避免內聯後相同的 20 日查詢被執行兩次。
valued AS MATERIALIZED (
    SELECT f.*,
           CASE WHEN $15::int8 IS NOT NULL OR $16::text = 'trade_value' OR $17::text <> 'equal' THEN (
               SELECT AVG(q."TradeValue")::float8 FROM "DailyQuotes" q
               WHERE q.stock_symbol = f.stock_symbol
                 AND q."Date" BETWEEN f.liquidity_start AND f.date
           ) END AS trade_value
    FROM filtered f
),
picks AS (
    SELECT date, stock_symbol, ret, stock_industry_id, trade_value,
           ROW_NUMBER() OVER (
               PARTITION BY date
               ORDER BY CASE $16::text
                            WHEN 'trade_value' THEN trade_value
                            WHEN 'foreign_streak' THEN foreign_buy
                            WHEN 'trust_streak' THEN trust_buy
                            WHEN 'revenue_yoy' THEN revenue_yoy
                            WHEN 'operating_margin' THEN operating_margin
                            WHEN 'pe_ratio' THEN -pe
                            WHEN 'pb_ratio' THEN -pb
                        END DESC NULLS LAST,
                        "#, rs_rank_order!(), r#"
           ) AS pick_rank
    FROM valued
    WHERE $15::int8 IS NULL OR trade_value >= $15
),
candidates AS (
    SELECT date,
           array_agg(stock_symbol::text ORDER BY pick_rank) AS symbols,
           array_agg(ret::float8 ORDER BY pick_rank) AS returns,
           array_agg(stock_industry_id::int4 ORDER BY pick_rank) AS industries,
           array_agg(trade_value ORDER BY pick_rank) AS trade_values
    FROM picks
    WHERE pick_rank <= $5
    GROUP BY date
),
holdable AS (
    SELECT date,
           array_agg(stock_symbol::text ORDER BY "#, rs_rank_order!(), r#") AS symbols,
           array_agg(ret::float8 ORDER BY "#, rs_rank_order!(), r#") AS returns,
           array_agg(stock_industry_id::int4 ORDER BY "#, rs_rank_order!(), r#") AS industries,
           array_agg(trade_value ORDER BY "#, rs_rank_order!(), r#") AS trade_values
    FROM (
        SELECT u.date, u.stock_symbol, u.rs_score, u.rs_rank, u.ret, u.stock_industry_id,
               CASE WHEN $17::text <> 'equal' THEN (
                   SELECT AVG(q."TradeValue")::float8 FROM "DailyQuotes" q
                   WHERE q.stock_symbol = u.stock_symbol
                     AND q."Date" BETWEEN u.liquidity_start AND u.date
               ) END AS trade_value
        FROM universe u
        WHERE $12::smallint IS NOT NULL AND u.rs_score >= $12
    ) AS held
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
),
valuation_level AS (
    SELECT cur.date,
           ($33::float8 IS NULL OR cur.median_pe::float8 <= $33::float8)
               AND ($34::int IS NULL OR h.samples < 250 OR h.below * 100 < $34::int * h.samples) AS cheap
    FROM daily_market_valuation cur
    CROSS JOIN LATERAL (
        SELECT COUNT(*) AS samples, COUNT(*) FILTER (WHERE p.median_pe < cur.median_pe) AS below
        FROM daily_market_valuation p
        WHERE $34::int IS NOT NULL
          AND p.stock_exchange_market_id = cur.stock_exchange_market_id
          AND p.date < cur.date AND p.date >= cur.date - INTERVAL '5 years'
    ) h
    WHERE ($33::float8 IS NOT NULL OR $34::int IS NOT NULL)
      AND cur.stock_exchange_market_id = $3 AND cur.date IN (SELECT date FROM rebal)
)
SELECT b.date, b.entry_date,
       b.exit_date,
       (b.exit_date IS NULL OR b.exit_date > $2 OR b.samples = 0) AS pending,
       (SELECT COUNT(*) FROM days d WHERE d.date > b.date AND d.date <= $2)::int8 AS elapsed_days,
       ($11::int IS NULL OR t.above IS NOT FALSE)
           AND ($13::int IS NULL OR br.above IS NOT FALSE)
           AND (($33::float8 IS NULL AND $34::int IS NULL) OR vl.cheap IS NOT FALSE) AS market_ok,
       b.avg_ret AS benchmark_return,
       CASE
           WHEN b.exit_date IS NOT NULL AND b.exit_date <= $2 THEN
               ("#,
            $etf_ret,
            r#")::float8
           ELSE NULL
       END AS etf_return,
       COALESCE(c.symbols, ARRAY[]::text[]) AS candidates,
       COALESCE(c.returns, ARRAY[]::float8[]) AS candidate_returns,
       COALESCE(c.industries, ARRAY[]::int4[]) AS candidate_industries,
       COALESCE(c.trade_values, ARRAY[]::float8[]) AS candidate_trade_values,
       COALESCE(h.symbols, ARRAY[]::text[]) AS hold_symbols,
       COALESCE(h.returns, ARRAY[]::float8[]) AS hold_returns,
       COALESCE(h.industries, ARRAY[]::int4[]) AS hold_industries,
       COALESCE(h.trade_values, ARRAY[]::float8[]) AS hold_trade_values
FROM bench b
LEFT JOIN candidates c ON c.date = b.date
LEFT JOIN holdable h ON h.date = b.date
LEFT JOIN daily_forward_return e ON e.date = b.entry_date AND e.stock_symbol = '0050'"#,
            $etf_joins,
            r#"
LEFT JOIN taiex t ON t."date" = b.date
LEFT JOIN breadth br ON br.date = b.date
LEFT JOIN valuation_level vl ON vl.date = b.date
WHERE (b.exit_date IS NOT NULL AND b.exit_date <= $2 AND b.samples > 0) OR b.date = (SELECT MAX(date) FROM rebal)
ORDER BY b.date
"#
        )
    };
}

/// 同天期報酬（`f` 為個股、`e` 為 0050 的 `daily_forward_return`）。
macro_rules! horizon_return {
    ($alias:literal) => {
        concat!(
            "CASE $4::int WHEN 5 THEN ",
            $alias,
            ".return_5d WHEN 20 THEN ",
            $alias,
            ".return_20d WHEN 60 THEN ",
            $alias,
            ".return_60d WHEN 120 THEN ",
            $alias,
            ".return_120d ELSE ",
            $alias,
            ".return_240d END"
        )
    };
}

/// 次日開盤版：收盤報酬乘上出場日、進場日的開盤價／收盤價比（`$exit`／`$entry` 為該日日線的別名）。
macro_rules! open_to_open {
    ($alias:literal, $entry:literal, $exit:literal) => {
        concat!(
            "((1 + (",
            horizon_return!($alias),
            ") / 100.0) * (CASE WHEN ",
            $exit,
            ".\"OpeningPrice\" > 0 AND ",
            $exit,
            ".\"ClosingPrice\" > 0 THEN ",
            $exit,
            ".\"OpeningPrice\" / ",
            $exit,
            ".\"ClosingPrice\" ELSE 1 END) / (CASE WHEN ",
            $entry,
            ".\"OpeningPrice\" > 0 AND ",
            $entry,
            ".\"ClosingPrice\" > 0 THEN ",
            $entry,
            ".\"OpeningPrice\" / ",
            $entry,
            ".\"ClosingPrice\" ELSE 1 END) - 1) * 100"
        )
    };
}

/// 依成交價模型（`close` 或 `next_open`）展開 [`backtest_sql!`]。
macro_rules! backtest_sql_for {
    (close, $with:literal, $chip_join:literal, $chip_filter:literal, $chip_columns:literal) => {
        backtest_sql!(
            $with,
            $chip_join,
            $chip_filter,
            $chip_columns,
            "date",
            "LEAD(date, $4::int) OVER (ORDER BY date)",
            horizon_return!("f"),
            "",
            horizon_return!("e"),
            ""
        )
    };
    (next_open, $with:literal, $chip_join:literal, $chip_filter:literal, $chip_columns:literal) => {
        backtest_sql!(
            $with,
            $chip_join,
            $chip_filter,
            $chip_columns,
            "LEAD(date, 1) OVER (ORDER BY date)",
            "LEAD(date, $4::int + 1) OVER (ORDER BY date)",
            open_to_open!("f", "qe", "qx"),
            "\n    LEFT JOIN \"DailyQuotes\" qe ON qe.stock_symbol = rs.stock_symbol AND qe.\"Date\" = rb.entry_date\n    LEFT JOIN \"DailyQuotes\" qx ON qx.stock_symbol = rs.stock_symbol AND qx.\"Date\" = rb.exit_date",
            open_to_open!("e", "ee", "ex"),
            "\nLEFT JOIN \"DailyQuotes\" ee ON ee.stock_symbol = '0050' AND ee.\"Date\" = b.entry_date\nLEFT JOIN \"DailyQuotes\" ex ON ex.stock_symbol = '0050' AND ex.\"Date\" = b.exit_date"
        )
    };
}

/// 籌碼版的 CTE 與條件（外資 `$37`、投信 `$38`）。
macro_rules! backtest_with_chip_sql_for {
    ($model:ident) => {
        concat!(
            "\nWITH",
            chip_streak_ctes!(),
            ",",
            backtest_sql_for!(
                $model,
                "",
                "\n    LEFT JOIN chip c ON c.\"date\" = u.date AND c.stock_symbol = u.stock_symbol",
                "\n      AND ($37::bigint IS NULL OR c.foreign_buy >= $37)\n      AND ($38::bigint IS NULL OR c.trust_buy >= $38)",
                "\n           COALESCE(c.foreign_buy, 0)::float8 AS foreign_buy,\n           COALESCE(c.trust_buy, 0)::float8 AS trust_buy"
            )
        )
    };
}

/// 沒有籌碼條件的回測。
pub(super) const BACKTEST_SQL: &str = backtest_sql_for!(
    close,
    "\nWITH",
    "",
    "",
    "\n           NULL::float8 AS foreign_buy, NULL::float8 AS trust_buy"
);

/// [`BACKTEST_SQL`] 的次日開盤成交版。
pub(super) const BACKTEST_NEXT_OPEN_SQL: &str = backtest_sql_for!(
    next_open,
    "\nWITH",
    "",
    "",
    "\n           NULL::float8 AS foreign_buy, NULL::float8 AS trust_buy"
);

/// 有籌碼條件（或依法人連續買超排序）的回測：換股當天外資（`$37`）／投信（`$38`）已連續買超至少
/// N 天，`NULL` 為不篩選；當天沒有籌碼資料的股票連續天數算 0，有設門檻時不會通過。
pub(super) const BACKTEST_WITH_CHIP_SQL: &str = backtest_with_chip_sql_for!(close);

/// [`BACKTEST_WITH_CHIP_SQL`] 的次日開盤成交版。
pub(super) const BACKTEST_WITH_CHIP_NEXT_OPEN_SQL: &str = backtest_with_chip_sql_for!(next_open);
