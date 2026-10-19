//! 個股事件時間軸的 SQL。

/// 個股事件時間軸。`$1` 代號、`$2`／`$3` 起訖、`$4` 已公告事件（除權息、股利發放、減資）列到這天、
/// `$5` 種類（`NULL` 為全部）、`$6` 筆數上限。
///
/// 每種事件整理成同一組欄位：`date`、`kind`、`period`、`label`、`note` 與數值 `v1`～`v3`（意義依種類，
/// 見 handler 的 [`super::describe`]）；每列另附 `count_kinds`／`count_values`（各種類的筆數，不受
/// `LIMIT` 影響）。`total` 是視窗函式，在 `LIMIT` 之前計算，也是總數。
///
/// - 股利：`dividend` 的除息日（`ex-dividend_date1`）、除權日（`ex-dividend_date2`）與發放日，非日期
///   字串（`-`）與金額為 0 的不列；`period` 為所屬年度加期別（例如 `2026Q2`）。
/// - 減資／分割：`corporate_action` 的生效日；`v1` 為換發比例。
/// - 月營收：以法定公布期限次月 10 日為事件日；`v1` 年增率、`v2` 月增率、`v3` 營收（千元）。
/// - 季報：以法定公布期限為事件日（Q1 5/15、Q2 8/14、Q3 11/14、Q4 隔年 3/31）；`v1` EPS、`v2` ROE。
/// - 技術訊號：`daily_technical_signal`；`label` 為來源、`note` 為說明。
/// - 法人連續買賣超：`chip_daily` 外資／投信的連續天數剛好為 ±5、±10、±20 的那天；`v1` 連續天數、
///   `v2` 當天買賣超股數。
/// - 董監持股：`insider_holding` 依月份加總，持股或質押比例較上月變化達 1%（質押比例為百分點）才列；
///   以申報期限次月 15 日為事件日；`v1` 持股變化（%）、`v2` 質押比例（%）、`v3` 質押比例變化（百分點）。
pub(super) const STOCK_TIMELINE_SQL: &str = r#"
WITH dividend_events AS (
    SELECT CASE WHEN e.raw_date ~ '^\d{4}-\d{2}-\d{2}$' THEN e.raw_date::date END AS date,
           e.kind,
           d.year_of_dividend::text || COALESCE(NULLIF(d.quarter, ''), '') AS period,
           e.amount
    FROM dividend d
    CROSS JOIN LATERAL (VALUES
        ('ex_dividend', d."ex-dividend_date1", d.cash_dividend),
        ('ex_right', d."ex-dividend_date2", d.stock_dividend),
        ('cash_payable', d.payable_date1, d.cash_dividend),
        ('stock_payable', d.payable_date2, d.stock_dividend)
    ) AS e(kind, raw_date, amount)
    WHERE d.security_code = $1
),
insider_months AS (
    SELECT month,
           SUM(shares)::numeric AS shares,
           SUM(pledged)::numeric AS pledged,
           LAG(SUM(shares)::numeric) OVER (ORDER BY month) AS prev_shares,
           LAG(SUM(pledged)::numeric) OVER (ORDER BY month) AS prev_pledged
    FROM insider_holding
    WHERE stock_symbol = $1 AND month BETWEEN $2::date - 400 AND $3
    GROUP BY month
),
events AS (
    SELECT date, kind, period, NULL::text AS label, NULL::text AS note,
           amount AS v1, NULL::numeric AS v2, NULL::numeric AS v3
    FROM dividend_events
    WHERE date BETWEEN $2 AND $4 AND amount > 0
  UNION ALL
    SELECT effective_date, 'corporate_action', NULL, action_type, note, share_ratio, NULL, NULL
    FROM corporate_action
    WHERE stock_symbol = $1 AND effective_date BETWEEN $2 AND $4
  UNION ALL
    SELECT event_date, 'monthly_revenue', period, NULL, NULL, yoy, mom, monthly
    FROM (
        SELECT (make_date(("Date" / 100)::int, ("Date" % 100)::int, 10) + INTERVAL '1 month')::date
                   AS event_date,
               to_char(make_date(("Date" / 100)::int, ("Date" % 100)::int, 1), 'YYYY-MM') AS period,
               "ComparedWithLastYearSameMonth" AS yoy,
               "ComparedWithLastMonth" AS mom,
               "Monthly" AS monthly
        FROM "Revenue"
        WHERE "SecurityCode" = $1
          AND "Date" BETWEEN EXTRACT(YEAR FROM $2::date - 40)::int * 100 + EXTRACT(MONTH FROM $2::date - 40)::int
                         AND EXTRACT(YEAR FROM $3::date)::int * 100 + EXTRACT(MONTH FROM $3::date)::int
    ) revenue
    WHERE event_date BETWEEN $2 AND $3
  UNION ALL
    SELECT event_date, 'quarterly_report', period, NULL, NULL, earnings_per_share, return_on_equity, NULL
    FROM (
        SELECT CASE quarter
                   WHEN 'Q1' THEN make_date(year::int, 5, 15)
                   WHEN 'Q2' THEN make_date(year::int, 8, 14)
                   WHEN 'Q3' THEN make_date(year::int, 11, 14)
                   ELSE make_date(year::int + 1, 3, 31)
               END AS event_date,
               year::text || quarter AS period,
               earnings_per_share,
               return_on_equity
        FROM financial_statement
        WHERE security_code = $1
          AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
          AND year BETWEEN EXTRACT(YEAR FROM $2::date)::int - 1 AND EXTRACT(YEAR FROM $3::date)::int
    ) report
    WHERE event_date BETWEEN $2 AND $3
  UNION ALL
    SELECT date, 'signal', NULL, indicator, description, NULL, NULL, NULL
    FROM daily_technical_signal
    WHERE stock_symbol = $1 AND date BETWEEN $2 AND $3
  UNION ALL
    SELECT c.date, 'chip_streak', NULL, s.investor, NULL, s.streak, s.net, NULL
    FROM chip_daily c
    CROSS JOIN LATERAL (VALUES
        ('foreign', c.foreign_streak::numeric, c.foreign_net::numeric),
        ('trust', c.trust_streak::numeric, c.trust_net::numeric)
    ) AS s(investor, streak, net)
    WHERE c.stock_symbol = $1 AND c.date BETWEEN $2 AND $3
      AND ABS(s.streak) IN (5, 10, 20)
  UNION ALL
    SELECT (month + INTERVAL '1 month' + INTERVAL '14 days')::date, 'insider', to_char(month, 'YYYY-MM'),
           NULL, NULL,
           (shares / prev_shares - 1) * 100,
           pledged / shares * 100,
           (pledged / shares - prev_pledged / prev_shares) * 100
    FROM insider_months
    WHERE prev_shares > 0 AND shares > 0
      AND (ABS(shares / prev_shares - 1) >= 0.01
           OR ABS(pledged / shares - prev_pledged / prev_shares) >= 0.01)
      AND (month + INTERVAL '1 month' + INTERVAL '14 days')::date BETWEEN $2 AND $3
),
matched AS (
    SELECT * FROM events WHERE $5::text[] IS NULL OR kind = ANY($5)
),
counted AS (
    SELECT array_agg(kind ORDER BY kind) AS kinds, array_agg(n ORDER BY kind) AS counts
    FROM (SELECT kind, COUNT(*) AS n FROM matched GROUP BY kind) per_kind
)
SELECT m.date, m.kind, m.period, m.label, m.note, m.v1, m.v2, m.v3,
       COUNT(*) OVER () AS total, c.kinds AS count_kinds, c.counts AS count_values
FROM matched m
CROSS JOIN counted c
ORDER BY m.date DESC, m.kind
LIMIT $6
"#;
