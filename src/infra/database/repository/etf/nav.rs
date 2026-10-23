//! ETF 每日淨值：MIS 預估淨值入庫與前一營業日正式淨值回填、MoneyDJ 歷史回補、淨值序列、
//! 近幾日的申贖金額、最新單位數與近期折溢價（`etf_daily_nav`）。

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    domain::etf::{
        EtfNavHistoryPoint, EtfNavRecord, EtfWeeklyFlow, PremiumPoint, UnitsChangePoint,
    },
    domain::quote::technical::PriceBar,
    infra::database,
};

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

/// 多檔 ETF 自 `$2` 起的淨值序列：正式淨值優先，當天還沒回填時用預估淨值。
const NAV_SERIES_SQL: &str = r#"
SELECT stock_symbol::text, date, COALESCE(nav, estimated_nav)::float8 AS value
FROM etf_daily_nav
WHERE stock_symbol = ANY($1) AND date >= $2 AND COALESCE(nav, estimated_nav) > 0
ORDER BY stock_symbol, date
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

pub(super) async fn save_navs(records: &[EtfNavRecord]) -> Result<u64> {
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

    let previous: Vec<&&EtfNavRecord> = rows.iter().filter(|r| r.previous_nav.is_some()).collect();
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

pub(super) async fn insert_nav_history(
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

pub(super) async fn nav_series(
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

pub(super) async fn weekly_flows(days: i64) -> Result<Vec<EtfWeeklyFlow>> {
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

pub(super) async fn latest_units(
    stock_symbols: &[String],
) -> Result<HashMap<String, UnitsChangePoint>> {
    let rows: Vec<(String, NaiveDate, i64, i64, Option<f64>)> = sqlx::query_as(LATEST_UNITS_SQL)
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

pub(super) async fn recent_premiums(
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
