//! ETF 含息績效（`etf_performance`）與清單頁快照（`etf_snapshot`）。

use anyhow::{Context, Result};
use rust_decimal::Decimal;

use crate::{
    domain::etf::{EtfNavReturns, EtfPerformance},
    infra::database,
};

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

/// f64 轉成 4 位小數的 `Decimal`；NaN、無限大為 `None`。
pub(super) fn to_decimal(value: Option<f64>) -> Option<Decimal> {
    value
        .filter(|v| v.is_finite())
        .and_then(Decimal::from_f64_retain)
        .map(|v| v.round_dp(4))
}

pub(super) async fn save_performances(performances: &[EtfPerformance]) -> Result<u64> {
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
    crate::infra::cache::etf_analysis::ETF_ANALYSIS.invalidate();
    Ok(written)
}

pub(super) async fn refresh_snapshot() -> Result<u64> {
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
    crate::infra::cache::etf_analysis::ETF_ANALYSIS.invalidate();
    Ok(written)
}
