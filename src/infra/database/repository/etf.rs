//! ETF 專區的 PostgreSQL 倉儲實作。
//!
//! 對應 `etf_daily_nav`、`etf_profile`、`etf_holding`、`etf_allocation`、`etf_performance`、
//! `etf_snapshot`（見 `etc/sql/etf.sql`）。

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    domain::etf::{
        EtfAllocationRecord, EtfHoldingPeriod, EtfHoldingPeriods, EtfHoldingRecord,
        EtfNavHistoryPoint, EtfNavRecord, EtfNavReturns, EtfPerformance, EtfPortfolio,
        EtfProfileRecord, EtfRepository, EtfWeeklyFlow, PremiumPoint, UnitsChangePoint,
    },
    domain::quote::technical::PriceBar,
    infra::database,
};

/// 基於 PostgreSQL 的 ETF 倉儲。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgEtfRepository;

impl PgEtfRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

const LISTED_ETFS_SQL: &str = r#"
SELECT stock_symbol::text, "Name"::text
FROM stocks
WHERE stock_industry_id = 9001 AND NOT "SuspendListing"
ORDER BY stock_symbol
"#;

const SAVE_NAVS_SQL: &str = r#"
INSERT INTO etf_daily_nav (date, stock_symbol, outstanding_units, units_change, market_price,
                           estimated_nav, premium_rate, source_time)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::bigint[], $4::bigint[], $5::numeric[],
                     $6::numeric[], $7::numeric[], $8::timestamptz[])
ON CONFLICT (date, stock_symbol) DO UPDATE SET
    outstanding_units = EXCLUDED.outstanding_units,
    units_change = EXCLUDED.units_change,
    market_price = EXCLUDED.market_price,
    estimated_nav = EXCLUDED.estimated_nav,
    premium_rate = EXCLUDED.premium_rate,
    source_time = EXCLUDED.source_time,
    updated_at = NOW()
WHERE (etf_daily_nav.outstanding_units, etf_daily_nav.units_change, etf_daily_nav.market_price,
       etf_daily_nav.estimated_nav, etf_daily_nav.premium_rate, etf_daily_nav.source_time)
      IS DISTINCT FROM
      (EXCLUDED.outstanding_units, EXCLUDED.units_change, EXCLUDED.market_price,
       EXCLUDED.estimated_nav, EXCLUDED.premium_rate, EXCLUDED.source_time)
"#;

/// 以「前一營業日單位淨值」回填前一個交易日那一列的正式淨值；那天 MIS 沒抓到時補一列只有淨值的資料。
///
/// 前一個營業日取三份日曆中較晚的那天：加權指數、日報價（兩者是台股交易日，互為備援），以及
/// 這檔 ETF 自己的上一筆（台股休市、海外照常交易時，海外 ETF 當天也有淨值，例如 2026-10-23）。
/// 只看自己的上一筆，遇到前一天 MIS 沒抓到就會把淨值寫進更早一天；不能看整張表的日期，
/// 否則休市日只有海外 ETF 的那天，會被台股 ETF 當成前一個營業日。
const BACKFILL_NAV_SQL: &str = r#"
INSERT INTO etf_daily_nav (date, stock_symbol, nav)
SELECT p.date, b.stock_symbol, b.prev_nav
FROM UNNEST($1::date[], $2::varchar[], $3::numeric[]) AS b(date, stock_symbol, prev_nav)
CROSS JOIN LATERAL (
    SELECT GREATEST(
        (SELECT MAX(i."date") FROM "index" AS i WHERE i.category = 'TAIEX' AND i."date" < b.date),
        (SELECT MAX(q."Date") FROM "DailyQuotes" AS q WHERE q."Date" < b.date),
        (SELECT MAX(n.date) FROM etf_daily_nav AS n
         WHERE n.stock_symbol = b.stock_symbol AND n.date < b.date)
    ) AS date
) AS p
WHERE p.date IS NOT NULL
ON CONFLICT (date, stock_symbol) DO UPDATE SET nav = EXCLUDED.nav, updated_at = NOW()
WHERE etf_daily_nav.nav IS DISTINCT FROM EXCLUDED.nav
"#;

/// 回補歷史淨值：新增沒有的日期，折溢價以收盤市價對正式淨值計算。
/// 已有的列只補空欄：正式淨值為空就補上；市價與折溢價只補沒有單位數的列（回補或回填補出的列），
/// 折溢價以保留下來的淨值計算（MIS 回填的正式淨值優先）；MIS 寫入的市價、預估折溢價一律不動。
const INSERT_NAV_HISTORY_SQL: &str = r#"
INSERT INTO etf_daily_nav (date, stock_symbol, market_price, nav, premium_rate)
SELECT h.date, $1, h.market_price, h.nav,
       CASE WHEN h.market_price IS NOT NULL THEN ROUND((h.market_price - h.nav) / h.nav * 100, 4) END
FROM UNNEST($2::date[], $3::numeric[], $4::numeric[]) AS h(date, nav, market_price)
ON CONFLICT (date, stock_symbol) DO UPDATE SET
    nav = COALESCE(etf_daily_nav.nav, EXCLUDED.nav),
    market_price = CASE WHEN etf_daily_nav.outstanding_units IS NULL
                        THEN COALESCE(etf_daily_nav.market_price, EXCLUDED.market_price)
                        ELSE etf_daily_nav.market_price END,
    premium_rate = CASE WHEN etf_daily_nav.outstanding_units IS NULL
                        THEN COALESCE(etf_daily_nav.premium_rate,
                                      ROUND((COALESCE(etf_daily_nav.market_price, EXCLUDED.market_price)
                                             - COALESCE(etf_daily_nav.nav, EXCLUDED.nav))
                                            / COALESCE(etf_daily_nav.nav, EXCLUDED.nav) * 100, 4))
                        ELSE etf_daily_nav.premium_rate END,
    updated_at = NOW()
WHERE etf_daily_nav.nav IS NULL
   OR (etf_daily_nav.outstanding_units IS NULL AND etf_daily_nav.market_price IS NULL
       AND EXCLUDED.market_price IS NOT NULL)
"#;

const SAVE_PROFILE_SQL: &str = r#"
INSERT INTO etf_profile (stock_symbol, full_name, issuer, inception_date, benchmark, category, is_active,
                         region, strategy, themes, dividend_frequency, management_fee, custodian_fee,
                         currency, exchange_name, manager, website, total_assets, total_assets_date,
                         nav_return, source)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)
ON CONFLICT (stock_symbol) DO UPDATE SET
    full_name = EXCLUDED.full_name,
    issuer = EXCLUDED.issuer,
    inception_date = EXCLUDED.inception_date,
    benchmark = EXCLUDED.benchmark,
    category = EXCLUDED.category,
    is_active = EXCLUDED.is_active,
    region = EXCLUDED.region,
    strategy = EXCLUDED.strategy,
    themes = EXCLUDED.themes,
    dividend_frequency = EXCLUDED.dividend_frequency,
    management_fee = EXCLUDED.management_fee,
    custodian_fee = EXCLUDED.custodian_fee,
    currency = EXCLUDED.currency,
    exchange_name = EXCLUDED.exchange_name,
    manager = EXCLUDED.manager,
    website = EXCLUDED.website,
    total_assets = EXCLUDED.total_assets,
    total_assets_date = EXCLUDED.total_assets_date,
    nav_return = EXCLUDED.nav_return,
    source = EXCLUDED.source,
    updated_at = NOW()
"#;

/// 推斷的分類只補沒有資料的 ETF；已有的推斷列跟著最新的推斷規則更新，Yahoo 寫入的列不動。
const INSERT_MISSING_PROFILE_SQL: &str = r#"
INSERT INTO etf_profile (stock_symbol, full_name, region, strategy, themes, source)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (stock_symbol) DO UPDATE SET
    region = EXCLUDED.region,
    strategy = EXCLUDED.strategy,
    themes = EXCLUDED.themes,
    updated_at = NOW()
WHERE etf_profile.source = 'heuristic'
  AND (etf_profile.region, etf_profile.strategy, etf_profile.themes)
      IS DISTINCT FROM (EXCLUDED.region, EXCLUDED.strategy, EXCLUDED.themes)
"#;

const SAVE_PERFORMANCES_SQL: &str = r#"
INSERT INTO etf_performance (stock_symbol, calc_date, ret_1w, ret_1m, ret_3m, ret_6m, ret_ytd, ret_1y,
                             ret_2y, ret_3y, ret_5y, ret_10y, vol_1y, mdd_1y, mdd_date, beta_1y,
                             sharpe_1y, year_low, year_high, nav_calc_date, nav_ret_1w, nav_ret_1m,
                             nav_ret_3m, nav_ret_6m, nav_ret_ytd, nav_ret_1y, nav_ret_2y, nav_ret_3y,
                             nav_ret_5y, nav_ret_10y)
SELECT * FROM UNNEST($1::varchar[], $2::date[], $3::numeric[], $4::numeric[], $5::numeric[], $6::numeric[],
                     $7::numeric[], $8::numeric[], $9::numeric[], $10::numeric[], $11::numeric[],
                     $12::numeric[], $13::numeric[], $14::numeric[], $15::date[], $16::numeric[],
                     $17::numeric[], $18::numeric[], $19::numeric[], $20::date[], $21::numeric[],
                     $22::numeric[], $23::numeric[], $24::numeric[], $25::numeric[], $26::numeric[],
                     $27::numeric[], $28::numeric[], $29::numeric[], $30::numeric[])
ON CONFLICT (stock_symbol) DO UPDATE SET
    calc_date = EXCLUDED.calc_date,
    ret_1w = EXCLUDED.ret_1w, ret_1m = EXCLUDED.ret_1m, ret_3m = EXCLUDED.ret_3m,
    ret_6m = EXCLUDED.ret_6m, ret_ytd = EXCLUDED.ret_ytd, ret_1y = EXCLUDED.ret_1y,
    ret_2y = EXCLUDED.ret_2y, ret_3y = EXCLUDED.ret_3y, ret_5y = EXCLUDED.ret_5y,
    ret_10y = EXCLUDED.ret_10y, vol_1y = EXCLUDED.vol_1y, mdd_1y = EXCLUDED.mdd_1y,
    mdd_date = EXCLUDED.mdd_date, beta_1y = EXCLUDED.beta_1y, sharpe_1y = EXCLUDED.sharpe_1y,
    year_low = EXCLUDED.year_low, year_high = EXCLUDED.year_high,
    nav_calc_date = EXCLUDED.nav_calc_date, nav_ret_1w = EXCLUDED.nav_ret_1w,
    nav_ret_1m = EXCLUDED.nav_ret_1m, nav_ret_3m = EXCLUDED.nav_ret_3m, nav_ret_6m = EXCLUDED.nav_ret_6m,
    nav_ret_ytd = EXCLUDED.nav_ret_ytd, nav_ret_1y = EXCLUDED.nav_ret_1y, nav_ret_2y = EXCLUDED.nav_ret_2y,
    nav_ret_3y = EXCLUDED.nav_ret_3y, nav_ret_5y = EXCLUDED.nav_ret_5y, nav_ret_10y = EXCLUDED.nav_ret_10y,
    updated_at = NOW()
"#;

/// 多檔 ETF 自 `$2` 起的淨值序列：正式淨值優先，當天還沒回填時用預估淨值。
const NAV_SERIES_SQL: &str = r#"
SELECT stock_symbol::text, date, COALESCE(nav, estimated_nav)::float8 AS value
FROM etf_daily_nav
WHERE stock_symbol = ANY($1) AND date >= $2 AND COALESCE(nav, estimated_nav) > 0
ORDER BY stock_symbol, date
"#;

/// 清單頁快照：每檔上市櫃 ETF 一列，彙整行情、淨值、分類、配息、績效、受益人數與外資買賣超。
///
/// - 分類以 `manual_override` 優先；配息頻率來源沒給時，以近 12 個月除息次數推得。
/// - 除息日欄位是文字，以 CASE 擋掉 `-` 等非日期值再轉型；同一天多列股利取最大值
///   （債券 ETF 有月配與季配兩列記同一筆的情形，與還原日線的規則相同）。
/// - 「近 12 個月」以每檔 ETF 自己的最新行情日為基準（沒有行情時用今天）。
const REFRESH_SNAPSHOT_SQL: &str = r#"
WITH etf AS (
    SELECT s.stock_symbol, replace(s."Name", '&amp;', '&') AS name, s.stock_exchange_market_id AS market_id,
           COALESCE(q.date, CURRENT_DATE) AS ref_date
    FROM stocks AS s
    LEFT JOIN last_daily_quotes AS q ON q.stock_symbol = s.stock_symbol
    WHERE s.stock_industry_id = 9001 AND NOT s."SuspendListing"
),
nav AS (
    SELECT DISTINCT ON (n.stock_symbol) n.stock_symbol, n.date, n.source_time, n.estimated_nav, n.nav,
           n.premium_rate, n.outstanding_units, n.units_change
    FROM etf_daily_nav AS n JOIN etf USING (stock_symbol)
    ORDER BY n.stock_symbol, n.date DESC
),
prem AS (
    SELECT stock_symbol, AVG(premium_rate) AS avg20
    FROM (SELECT n.stock_symbol, n.premium_rate,
                 ROW_NUMBER() OVER (PARTITION BY n.stock_symbol ORDER BY n.date DESC) AS rn
          FROM etf_daily_nav AS n JOIN etf USING (stock_symbol)
          WHERE n.premium_rate IS NOT NULL) AS t
    WHERE rn <= 20
    GROUP BY stock_symbol
),
div_events AS (
    SELECT stock_symbol, ref_date, ex_date, MAX(cash) AS cash
    FROM (SELECT d.security_code AS stock_symbol, etf.ref_date,
                 CASE WHEN d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$'
                      THEN d."ex-dividend_date1"::date END AS ex_date,
                 d.cash_dividend AS cash
          FROM dividend AS d JOIN etf ON etf.stock_symbol = d.security_code
          WHERE d.cash_dividend > 0) AS e
    WHERE ex_date IS NOT NULL
    GROUP BY stock_symbol, ref_date, ex_date
),
div12 AS (
    SELECT stock_symbol, SUM(cash) AS total, COUNT(*) AS cnt
    FROM div_events
    WHERE ex_date > (ref_date - INTERVAL '1 year')::date AND ex_date <= ref_date
    GROUP BY stock_symbol
),
last_div AS (
    SELECT DISTINCT ON (stock_symbol) stock_symbol, ex_date, cash
    FROM div_events
    WHERE ex_date <= ref_date
    ORDER BY stock_symbol, ex_date DESC
),
next_div AS (
    SELECT DISTINCT ON (stock_symbol) stock_symbol, ex_date, cash
    FROM div_events
    WHERE ex_date > ref_date
    ORDER BY stock_symbol, ex_date
),
holders AS (
    SELECT DISTINCT ON (h.stock_symbol) h.stock_symbol, h.total_holders, h.date
    FROM holder_distribution AS h JOIN etf USING (stock_symbol)
    ORDER BY h.stock_symbol, h.date DESC
),
chip AS (
    SELECT DISTINCT ON (c.stock_symbol) c.stock_symbol, c.foreign_net, c.date
    FROM chip_daily AS c JOIN etf USING (stock_symbol)
    WHERE c.foreign_net IS NOT NULL
    ORDER BY c.stock_symbol, c.date DESC
),
rows AS (
    SELECT etf.stock_symbol,
           etf.name,
           COALESCE(p.full_name, '') AS full_name,
           etf.market_id,
           COALESCE(NULLIF(p.manual_override ->> 'strategy', ''), p.strategy, '') AS strategy,
           COALESCE(NULLIF(p.manual_override ->> 'region', ''), p.region, '') AS region,
           COALESCE(CASE WHEN jsonb_typeof(p.manual_override -> 'themes') = 'array'
                         THEN ARRAY(SELECT jsonb_array_elements_text(p.manual_override -> 'themes')) END,
                    p.themes, '{}') AS themes,
           COALESCE(NULLIF(COALESCE(p.manual_override ->> 'dividend_frequency', p.dividend_frequency), ''),
                    CASE WHEN div12.cnt >= 10 THEN 'monthly'
                         WHEN div12.cnt >= 3 THEN 'quarterly'
                         WHEN div12.cnt = 2 THEN 'semi_annual'
                         WHEN div12.cnt = 1 THEN 'annual'
                         WHEN p.source = 'yahoo' THEN 'none'
                         ELSE '' END) AS dividend_frequency,
           COALESCE(p.issuer, '') AS issuer,
           COALESCE(p.benchmark, '') AS benchmark,
           q.date AS quote_date,
           q.closing_price,
           q.change,
           q.change_range,
           ROUND(q.trading_volume / 1000)::bigint AS trading_volume,
           q.trade_value,
           nav.date AS nav_date,
           nav.source_time AS nav_time,
           nav.estimated_nav AS nav,
           nav.premium_rate,
           ROUND(prem.avg20, 4) AS premium_avg_20d,
           nav.outstanding_units,
           COALESCE(ROUND(nav.outstanding_units * COALESCE(nav.estimated_nav, nav.nav), 2),
                    p.total_assets) AS total_assets,
           nav.units_change,
           ROUND(nav.units_change * COALESCE(nav.estimated_nav, nav.nav), 2) AS net_flow,
           div12.total AS dividend_12m,
           div12.cnt::integer AS dividend_count_12m,
           ROUND(div12.total / NULLIF(q.closing_price, 0) * 100, 4) AS yield_12m,
           last_div.cash AS last_dividend,
           last_div.ex_date AS last_ex_date,
           next_div.cash AS next_dividend,
           next_div.ex_date AS next_ex_date,
           perf.ret_1w, perf.ret_1m, perf.ret_3m, perf.ret_6m, perf.ret_ytd, perf.ret_1y,
           perf.year_low, perf.year_high,
           holders.total_holders AS holders,
           holders.date AS holders_date,
           chip.foreign_net,
           chip.date AS chip_date
    FROM etf
    LEFT JOIN etf_profile AS p ON p.stock_symbol = etf.stock_symbol
    LEFT JOIN last_daily_quotes AS q ON q.stock_symbol = etf.stock_symbol
    LEFT JOIN nav ON nav.stock_symbol = etf.stock_symbol
    LEFT JOIN prem ON prem.stock_symbol = etf.stock_symbol
    LEFT JOIN div12 ON div12.stock_symbol = etf.stock_symbol
    LEFT JOIN last_div ON last_div.stock_symbol = etf.stock_symbol
    LEFT JOIN next_div ON next_div.stock_symbol = etf.stock_symbol
    LEFT JOIN etf_performance AS perf ON perf.stock_symbol = etf.stock_symbol
    LEFT JOIN holders ON holders.stock_symbol = etf.stock_symbol
    LEFT JOIN chip ON chip.stock_symbol = etf.stock_symbol
)
INSERT INTO etf_snapshot (stock_symbol, name, full_name, market_id, strategy, region, themes,
                          dividend_frequency, issuer, benchmark, quote_date, closing_price, change,
                          change_range, trading_volume, trade_value, nav_date, nav_time, nav,
                          premium_rate, premium_avg_20d, outstanding_units, total_assets, units_change,
                          net_flow, dividend_12m, dividend_count_12m, yield_12m, last_dividend,
                          last_ex_date, next_dividend, next_ex_date, ret_1w, ret_1m, ret_3m, ret_6m,
                          ret_ytd, ret_1y, year_low, year_high, holders, holders_date, foreign_net,
                          chip_date)
SELECT * FROM rows
ON CONFLICT (stock_symbol) DO UPDATE SET
    name = EXCLUDED.name, full_name = EXCLUDED.full_name, market_id = EXCLUDED.market_id,
    strategy = EXCLUDED.strategy, region = EXCLUDED.region, themes = EXCLUDED.themes,
    dividend_frequency = EXCLUDED.dividend_frequency, issuer = EXCLUDED.issuer,
    benchmark = EXCLUDED.benchmark, quote_date = EXCLUDED.quote_date,
    closing_price = EXCLUDED.closing_price, change = EXCLUDED.change,
    change_range = EXCLUDED.change_range, trading_volume = EXCLUDED.trading_volume,
    trade_value = EXCLUDED.trade_value, nav_date = EXCLUDED.nav_date, nav_time = EXCLUDED.nav_time,
    nav = EXCLUDED.nav, premium_rate = EXCLUDED.premium_rate,
    premium_avg_20d = EXCLUDED.premium_avg_20d, outstanding_units = EXCLUDED.outstanding_units,
    total_assets = EXCLUDED.total_assets, units_change = EXCLUDED.units_change,
    net_flow = EXCLUDED.net_flow, dividend_12m = EXCLUDED.dividend_12m,
    dividend_count_12m = EXCLUDED.dividend_count_12m, yield_12m = EXCLUDED.yield_12m,
    last_dividend = EXCLUDED.last_dividend, last_ex_date = EXCLUDED.last_ex_date,
    next_dividend = EXCLUDED.next_dividend, next_ex_date = EXCLUDED.next_ex_date,
    ret_1w = EXCLUDED.ret_1w, ret_1m = EXCLUDED.ret_1m, ret_3m = EXCLUDED.ret_3m,
    ret_6m = EXCLUDED.ret_6m, ret_ytd = EXCLUDED.ret_ytd, ret_1y = EXCLUDED.ret_1y,
    year_low = EXCLUDED.year_low, year_high = EXCLUDED.year_high, holders = EXCLUDED.holders,
    holders_date = EXCLUDED.holders_date, foreign_net = EXCLUDED.foreign_net,
    chip_date = EXCLUDED.chip_date, updated_at = NOW()
"#;

const DELETE_STALE_SNAPSHOT_SQL: &str = r#"
DELETE FROM etf_snapshot AS s
WHERE NOT EXISTS (SELECT 1 FROM stocks AS t
                  WHERE t.stock_symbol = s.stock_symbol
                    AND t.stock_industry_id = 9001 AND NOT t."SuspendListing")
"#;

const RECENT_PREMIUMS_SQL: &str = r#"
SELECT stock_symbol::text, date, premium_rate
FROM (SELECT stock_symbol, date, premium_rate,
             ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY date DESC) AS rn
      FROM etf_daily_nav
      WHERE stock_symbol = ANY($1) AND premium_rate IS NOT NULL) AS t
WHERE rn <= $2
ORDER BY stock_symbol, date DESC
"#;

/// 指定 ETF 最新兩期的前十大持股；只取最新一期在 `$2`（含）之後才收到、且有前一期的。
const HOLDING_PERIODS_SQL: &str = r#"
WITH periods AS (
    SELECT stock_symbol, as_of, MIN(recorded_at) AS recorded_at,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY as_of DESC) AS rn
    FROM etf_holding_history
    WHERE stock_symbol = ANY($1)
    GROUP BY stock_symbol, as_of
),
fresh AS (
    SELECT p.stock_symbol FROM periods AS p
    WHERE p.rn = 1 AND p.recorded_at >= $2::date
      AND EXISTS (SELECT 1 FROM periods AS q WHERE q.stock_symbol = p.stock_symbol AND q.rn = 2)
)
SELECT h.stock_symbol::text AS stock_symbol, p.rn, h.as_of, h.rank, h.holding_symbol::text AS holding_symbol,
       h.holding_name, h.weight
FROM etf_holding_history AS h
JOIN periods AS p ON p.stock_symbol = h.stock_symbol AND p.as_of = h.as_of
JOIN fresh AS f ON f.stock_symbol = h.stock_symbol
WHERE p.rn <= 2
ORDER BY h.stock_symbol, p.rn, h.rank
"#;

/// 最近 `$1` 個台股交易日（加權指數日曆）起有單位數的日子：每檔 ETF 的淨申購合計
/// （單位增減 × 當日淨值）與折溢價。期間內台股休市、只有海外 ETF 有資料的日子也算進去，
/// 但不佔交易日的名額。
const WEEKLY_FLOWS_SQL: &str = r#"
WITH d AS (
    SELECT DISTINCT date FROM etf_daily_nav
    WHERE units_change IS NOT NULL
      AND date >= (SELECT MIN(t."date")
                   FROM (SELECT i."date" FROM "index" AS i WHERE i.category = 'TAIEX'
                         ORDER BY i."date" DESC LIMIT $1) AS t)
)
SELECT n.stock_symbol::text AS stock_symbol, COALESCE(MAX(s.name), n.stock_symbol::text) AS name,
       COALESCE(SUM(n.units_change * COALESCE(n.estimated_nav, n.nav)), 0)::float8 AS net_flow,
       AVG(n.premium_rate)::float8 AS premium_avg,
       (ARRAY_AGG(n.premium_rate ORDER BY n.date DESC))[1]::float8 AS premium_last,
       MIN(n.date) AS from_date, MAX(n.date) AS to_date
FROM etf_daily_nav AS n
JOIN d USING (date)
LEFT JOIN etf_snapshot AS s ON s.stock_symbol = n.stock_symbol
WHERE n.units_change IS NOT NULL
GROUP BY n.stock_symbol
ORDER BY n.stock_symbol
"#;

/// 指定 ETF 最近一筆有單位數的資料（MIS 寫入的列）。
const LATEST_UNITS_SQL: &str = r#"
SELECT DISTINCT ON (stock_symbol) stock_symbol::text, date, outstanding_units, units_change,
       COALESCE(estimated_nav, nav)::float8 AS nav
FROM etf_daily_nav
WHERE stock_symbol = ANY($1) AND outstanding_units IS NOT NULL AND units_change IS NOT NULL
ORDER BY stock_symbol, date DESC
"#;

/// f64 轉成 4 位小數的 `Decimal`；NaN、無限大為 `None`。
fn to_decimal(value: Option<f64>) -> Option<Decimal> {
    value
        .filter(|v| v.is_finite())
        .and_then(Decimal::from_f64_retain)
        .map(|v| v.round_dp(4))
}

#[async_trait]
impl EtfRepository for PgEtfRepository {
    async fn listed_etfs(&self) -> Result<Vec<(String, String)>> {
        sqlx::query_as(LISTED_ETFS_SQL)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch listed ETFs")
    }

    async fn save_navs(&self, records: &[EtfNavRecord]) -> Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        // 同一批內主鍵重複時 ON CONFLICT 會報錯，以最後一筆為準。
        let mut unique: HashMap<(NaiveDate, &str), &EtfNavRecord> = HashMap::new();
        for record in records {
            unique.insert((record.date, record.stock_symbol.as_str()), record);
        }
        let rows: Vec<&EtfNavRecord> = unique.into_values().collect();
        let dates: Vec<NaiveDate> = rows.iter().map(|r| r.date).collect();
        let symbols: Vec<&str> = rows.iter().map(|r| r.stock_symbol.as_str()).collect();
        let units: Vec<i64> = rows.iter().map(|r| r.outstanding_units).collect();
        let changes: Vec<i64> = rows.iter().map(|r| r.units_change).collect();
        let prices: Vec<Option<Decimal>> = rows.iter().map(|r| r.market_price).collect();
        let navs: Vec<Option<Decimal>> = rows.iter().map(|r| r.estimated_nav).collect();
        let premiums: Vec<Option<Decimal>> = rows.iter().map(|r| r.premium_rate).collect();
        let times: Vec<Option<DateTime<Utc>>> = rows
            .iter()
            .map(|r| r.source_time.map(|t| t.with_timezone(&Utc)))
            .collect();

        let mut tx = database::get_tx().await?;
        let written = sqlx::query(SAVE_NAVS_SQL)
            .bind(&dates)
            .bind(&symbols)
            .bind(&units)
            .bind(&changes)
            .bind(&prices)
            .bind(&navs)
            .bind(&premiums)
            .bind(&times)
            .execute(&mut *tx)
            .await
            .context("Failed to save ETF navs")?
            .rows_affected();

        let previous: Vec<&&EtfNavRecord> =
            rows.iter().filter(|r| r.previous_nav.is_some()).collect();
        let prev_dates: Vec<NaiveDate> = previous.iter().map(|r| r.date).collect();
        let prev_symbols: Vec<&str> = previous.iter().map(|r| r.stock_symbol.as_str()).collect();
        let prev_navs: Vec<Option<Decimal>> = previous.iter().map(|r| r.previous_nav).collect();
        let backfilled = sqlx::query(BACKFILL_NAV_SQL)
            .bind(&prev_dates)
            .bind(&prev_symbols)
            .bind(&prev_navs)
            .execute(&mut *tx)
            .await
            .context("Failed to backfill ETF official navs")?
            .rows_affected();
        tx.commit().await?;
        Ok(written + backfilled)
    }

    async fn insert_nav_history(
        &self,
        stock_symbol: &str,
        points: &[EtfNavHistoryPoint],
    ) -> Result<u64> {
        if points.is_empty() {
            return Ok(0);
        }
        let dates: Vec<NaiveDate> = points.iter().map(|p| p.date).collect();
        let navs: Vec<Decimal> = points.iter().map(|p| p.nav).collect();
        let prices: Vec<Option<Decimal>> = points.iter().map(|p| p.market_price).collect();
        Ok(sqlx::query(INSERT_NAV_HISTORY_SQL)
            .bind(stock_symbol)
            .bind(&dates)
            .bind(&navs)
            .bind(&prices)
            .execute(database::get_connection())
            .await
            .context("Failed to insert ETF nav history")?
            .rows_affected())
    }

    async fn save_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64> {
        let mut tx = database::get_tx().await?;
        let mut written = 0;
        for p in profiles {
            written += sqlx::query(SAVE_PROFILE_SQL)
                .bind(&p.stock_symbol)
                .bind(&p.full_name)
                .bind(&p.issuer)
                .bind(p.inception_date)
                .bind(&p.benchmark)
                .bind(&p.category)
                .bind(p.is_active)
                .bind(&p.region)
                .bind(&p.strategy)
                .bind(&p.themes)
                .bind(&p.dividend_frequency)
                .bind(&p.management_fee)
                .bind(&p.custodian_fee)
                .bind(&p.currency)
                .bind(&p.exchange_name)
                .bind(&p.manager)
                .bind(&p.website)
                .bind(p.total_assets)
                .bind(p.total_assets_date)
                .bind(&p.nav_return)
                .bind(&p.source)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("Failed to save ETF profile {}", p.stock_symbol))?
                .rows_affected();
        }
        tx.commit().await?;
        Ok(written)
    }

    async fn insert_missing_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64> {
        let mut tx = database::get_tx().await?;
        let mut written = 0;
        for p in profiles {
            written += sqlx::query(INSERT_MISSING_PROFILE_SQL)
                .bind(&p.stock_symbol)
                .bind(&p.full_name)
                .bind(&p.region)
                .bind(&p.strategy)
                .bind(&p.themes)
                .bind(&p.source)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("Failed to insert ETF profile {}", p.stock_symbol))?
                .rows_affected();
        }
        tx.commit().await?;
        Ok(written)
    }

    async fn replace_portfolio(&self, portfolio: &EtfPortfolio) -> Result<u64> {
        let symbol = portfolio.stock_symbol.as_str();
        let mut tx = database::get_tx().await?;
        let mut written = 0;

        if let (Some(as_of), false) = (portfolio.holdings_as_of, portfolio.holdings.is_empty()) {
            sqlx::query("DELETE FROM etf_holding WHERE stock_symbol = $1")
                .bind(symbol)
                .execute(&mut *tx)
                .await?;
            for h in &portfolio.holdings {
                written += sqlx::query(
                    "INSERT INTO etf_holding (stock_symbol, rank, as_of, holding_symbol, holding_name, weight)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(symbol)
                .bind(h.rank)
                .bind(as_of)
                .bind(&h.holding_symbol)
                .bind(&h.holding_name)
                .bind(h.weight)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("Failed to save ETF holdings {symbol}"))?
                .rows_affected();
                // 每一期另存一份；同一期重抓只更正內容，保留第一次收到的時間。
                sqlx::query(
                    "INSERT INTO etf_holding_history (stock_symbol, as_of, rank, holding_symbol, holding_name, weight)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (stock_symbol, as_of, rank) DO UPDATE SET
                         holding_symbol = EXCLUDED.holding_symbol, holding_name = EXCLUDED.holding_name,
                         weight = EXCLUDED.weight",
                )
                .bind(symbol)
                .bind(as_of)
                .bind(h.rank)
                .bind(&h.holding_symbol)
                .bind(&h.holding_name)
                .bind(h.weight)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("Failed to save ETF holding history {symbol}"))?;
            }
        }

        for (kind, as_of, rows) in [
            (
                "industry",
                portfolio.industries_as_of,
                &portfolio.industries,
            ),
            ("asset", portfolio.assets_as_of, &portfolio.assets),
        ] {
            let Some(as_of) = as_of.filter(|_| !rows.is_empty()) else {
                continue;
            };
            written += replace_allocation(&mut tx, symbol, kind, as_of, rows).await?;
        }
        tx.commit().await?;
        Ok(written)
    }

    async fn nav_series(
        &self,
        stock_symbols: &[String],
        from: NaiveDate,
    ) -> Result<HashMap<String, Vec<PriceBar>>> {
        let rows: Vec<(String, NaiveDate, f64)> = sqlx::query_as(NAV_SERIES_SQL)
            .bind(stock_symbols)
            .bind(from)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch ETF nav series")?;
        let mut series: HashMap<String, Vec<PriceBar>> = HashMap::new();
        for (symbol, date, value) in rows {
            series.entry(symbol).or_default().push(PriceBar {
                date,
                close: value,
                high: value,
                low: value,
            });
        }
        Ok(series)
    }

    async fn save_performances(&self, performances: &[EtfPerformance]) -> Result<u64> {
        let rows: Vec<&EtfPerformance> = performances
            .iter()
            .filter(|p| p.calc_date.is_some())
            .collect();
        let symbols: Vec<&str> = rows.iter().map(|p| p.stock_symbol.as_str()).collect();
        let column = |f: fn(&EtfPerformance) -> Option<f64>| -> Vec<Option<Decimal>> {
            rows.iter().map(|p| to_decimal(f(p))).collect()
        };
        let nav_column = |f: fn(&EtfNavReturns) -> Option<f64>| -> Vec<Option<Decimal>> {
            rows.iter()
                .map(|p| to_decimal(p.nav_returns.as_ref().and_then(f)))
                .collect()
        };
        let mut tx = database::get_tx().await?;
        let mut written = 0;
        if !rows.is_empty() {
            written = sqlx::query(SAVE_PERFORMANCES_SQL)
                .bind(&symbols)
                .bind(rows.iter().map(|p| p.calc_date).collect::<Vec<_>>())
                .bind(column(|p| p.ret_1w))
                .bind(column(|p| p.ret_1m))
                .bind(column(|p| p.ret_3m))
                .bind(column(|p| p.ret_6m))
                .bind(column(|p| p.ret_ytd))
                .bind(column(|p| p.ret_1y))
                .bind(column(|p| p.ret_2y))
                .bind(column(|p| p.ret_3y))
                .bind(column(|p| p.ret_5y))
                .bind(column(|p| p.ret_10y))
                .bind(column(|p| p.vol_1y))
                .bind(column(|p| p.mdd_1y))
                .bind(rows.iter().map(|p| p.mdd_date).collect::<Vec<_>>())
                .bind(column(|p| p.beta_1y))
                .bind(column(|p| p.sharpe_1y))
                .bind(column(|p| p.year_low))
                .bind(column(|p| p.year_high))
                .bind(
                    rows.iter()
                        .map(|p| p.nav_returns.as_ref().map(|n| n.calc_date))
                        .collect::<Vec<_>>(),
                )
                .bind(nav_column(|n| n.ret_1w))
                .bind(nav_column(|n| n.ret_1m))
                .bind(nav_column(|n| n.ret_3m))
                .bind(nav_column(|n| n.ret_6m))
                .bind(nav_column(|n| n.ret_ytd))
                .bind(nav_column(|n| n.ret_1y))
                .bind(nav_column(|n| n.ret_2y))
                .bind(nav_column(|n| n.ret_3y))
                .bind(nav_column(|n| n.ret_5y))
                .bind(nav_column(|n| n.ret_10y))
                .execute(&mut *tx)
                .await
                .context("Failed to save ETF performances")?
                .rows_affected();
        }
        // 只刪已下市的 ETF；這批是空的（例如計算失敗）也不會清掉其他 ETF 的績效。
        written += sqlx::query(
            r#"DELETE FROM etf_performance AS p
               WHERE NOT EXISTS (SELECT 1 FROM stocks AS t
                                 WHERE t.stock_symbol = p.stock_symbol
                                   AND t.stock_industry_id = 9001 AND NOT t."SuspendListing")"#,
        )
        .execute(&mut *tx)
        .await
        .context("Failed to delete stale ETF performances")?
        .rows_affected();
        tx.commit().await?;
        Ok(written)
    }

    async fn refresh_snapshot(&self) -> Result<u64> {
        let mut tx = database::get_tx().await?;
        let written = sqlx::query(REFRESH_SNAPSHOT_SQL)
            .execute(&mut *tx)
            .await
            .context("Failed to refresh ETF snapshot")?
            .rows_affected();
        sqlx::query(DELETE_STALE_SNAPSHOT_SQL)
            .execute(&mut *tx)
            .await
            .context("Failed to delete stale ETF snapshot rows")?;
        tx.commit().await?;
        Ok(written)
    }

    async fn holding_periods(
        &self,
        stock_symbols: &[String],
        recorded_since: NaiveDate,
    ) -> Result<Vec<EtfHoldingPeriods>> {
        let rows: Vec<(String, i64, NaiveDate, i16, String, String, Decimal)> =
            sqlx::query_as(HOLDING_PERIODS_SQL)
                .bind(stock_symbols)
                .bind(recorded_since)
                .fetch_all(database::get_connection())
                .await
                .context("Failed to fetch ETF holding periods")?;
        let mut out: Vec<EtfHoldingPeriods> = Vec::new();
        for (symbol, rn, as_of, rank, holding_symbol, holding_name, weight) in rows {
            if out.last().is_none_or(|last| last.stock_symbol != symbol) {
                let empty = EtfHoldingPeriod {
                    as_of,
                    holdings: Vec::new(),
                };
                out.push(EtfHoldingPeriods {
                    stock_symbol: symbol.clone(),
                    previous: empty.clone(),
                    current: empty,
                });
            }
            let entry = out.last_mut().expect("剛放入");
            let period = if rn == 1 {
                &mut entry.current
            } else {
                &mut entry.previous
            };
            period.as_of = as_of;
            period.holdings.push(EtfHoldingRecord {
                rank,
                holding_symbol,
                holding_name,
                weight,
            });
        }
        Ok(out)
    }

    async fn weekly_flows(&self, days: i64) -> Result<Vec<EtfWeeklyFlow>> {
        let rows: Vec<WeeklyFlowRow> = sqlx::query_as(WEEKLY_FLOWS_SQL)
            .bind(days)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch ETF weekly flows")?;
        Ok(rows
            .into_iter()
            .map(|row| EtfWeeklyFlow {
                stock_symbol: row.stock_symbol,
                name: row.name,
                net_flow: row.net_flow,
                premium_avg: row.premium_avg,
                premium_last: row.premium_last,
                from: row.from_date,
                to: row.to_date,
            })
            .collect())
    }

    async fn latest_units(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, UnitsChangePoint>> {
        let rows: Vec<(String, NaiveDate, i64, i64, Option<f64>)> =
            sqlx::query_as(LATEST_UNITS_SQL)
                .bind(stock_symbols)
                .fetch_all(database::get_connection())
                .await
                .context("Failed to fetch ETF units")?;
        Ok(rows
            .into_iter()
            .map(|(symbol, date, outstanding_units, units_change, nav)| {
                (
                    symbol,
                    UnitsChangePoint {
                        date,
                        outstanding_units,
                        units_change,
                        nav,
                    },
                )
            })
            .collect())
    }

    async fn recent_premiums(
        &self,
        stock_symbols: &[String],
        days: i64,
    ) -> Result<HashMap<String, Vec<PremiumPoint>>> {
        let rows: Vec<(String, NaiveDate, Decimal)> = sqlx::query_as(RECENT_PREMIUMS_SQL)
            .bind(stock_symbols)
            .bind(days)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch ETF premiums")?;
        let mut out: HashMap<String, Vec<PremiumPoint>> = HashMap::new();
        for (symbol, date, premium) in rows {
            out.entry(symbol).or_default().push(PremiumPoint {
                date,
                premium_rate: premium.to_f64().unwrap_or_default(),
            });
        }
        Ok(out)
    }
}

/// [`WEEKLY_FLOWS_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct WeeklyFlowRow {
    stock_symbol: String,
    name: String,
    net_flow: f64,
    premium_avg: Option<f64>,
    premium_last: Option<f64>,
    from_date: NaiveDate,
    to_date: NaiveDate,
}

async fn replace_allocation(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    symbol: &str,
    kind: &str,
    as_of: NaiveDate,
    rows: &[EtfAllocationRecord],
) -> Result<u64> {
    sqlx::query("DELETE FROM etf_allocation WHERE stock_symbol = $1 AND kind = $2")
        .bind(symbol)
        .bind(kind)
        .execute(&mut **tx)
        .await?;
    let mut written = 0;
    // 同名的列（來源偶有重複）只留第一筆，避免主鍵衝突。
    let mut seen = std::collections::HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        if !seen.insert(row.label.as_str()) {
            continue;
        }
        written += sqlx::query(
            "INSERT INTO etf_allocation (stock_symbol, kind, label, as_of, weight, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(symbol)
        .bind(kind)
        .bind(&row.label)
        .bind(as_of)
        .bind(row.weight)
        .bind(i16::try_from(index).unwrap_or(i16::MAX))
        .execute(&mut **tx)
        .await
        .with_context(|| format!("Failed to save ETF {kind} allocation {symbol}"))?
        .rows_affected();
    }
    Ok(written)
}

#[cfg(test)]
mod tests;
