//! Data API 讀取 ETF 專區的 PostgreSQL 查詢：清單讀 `etf_snapshot`，明細另讀基本資料、績效、
//! 持股配置與近期淨值。資料都由排程預先整理，這裡只讀。

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow};

use crate::{
    domain::etf::{
        EtfHoldingPeriod, EtfHoldingRecord,
        query::{
            EtfDetail, EtfHoldingHistory, EtfListFilter, EtfNavDay, EtfOverview,
            EtfPerformanceView, EtfProfileView, EtfQueryRepository, EtfWeightRow,
        },
    },
    infra::database,
};

/// 基於 PostgreSQL 的 ETF 專區查詢。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgEtfQueryRepository;

impl PgEtfQueryRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

/// 清單與明細共用的概況欄位。
const OVERVIEW_COLUMNS: &str = r#"
SELECT s.stock_symbol::text AS stock_symbol, s.name, s.market_id, s.strategy, s.region, s.themes,
       s.dividend_frequency, s.issuer, s.benchmark, s.quote_date, s.closing_price::float8 AS closing_price,
       s.change_range::float8 AS change_range, s.trade_value::float8 AS trade_value, s.nav_date,
       s.nav::float8 AS nav, s.premium_rate::float8 AS premium_rate,
       s.premium_avg_20d::float8 AS premium_avg_20d, s.total_assets::float8 AS total_assets,
       s.net_flow::float8 AS net_flow, s.dividend_12m::float8 AS dividend_12m,
       s.yield_12m::float8 AS yield_12m, s.next_ex_date, s.next_dividend::float8 AS next_dividend,
       s.ret_1m::float8 AS ret_1m, s.ret_ytd::float8 AS ret_ytd, s.ret_1y::float8 AS ret_1y,
       s.holders, s.foreign_net"#;

const PROFILE_SQL: &str = r#"
SELECT full_name, inception_date, is_active, management_fee, custodian_fee, currency, source
FROM etf_profile WHERE stock_symbol = $1
"#;

const PERFORMANCE_SQL: &str = r#"
SELECT calc_date, ret_1w::float8 AS ret_1w, ret_1m::float8 AS ret_1m, ret_3m::float8 AS ret_3m,
       ret_6m::float8 AS ret_6m, ret_ytd::float8 AS ret_ytd, ret_1y::float8 AS ret_1y,
       ret_3y::float8 AS ret_3y, ret_5y::float8 AS ret_5y, vol_1y::float8 AS vol_1y,
       mdd_1y::float8 AS mdd_1y, beta_1y::float8 AS beta_1y, sharpe_1y::float8 AS sharpe_1y,
       nav_calc_date, nav_ret_1m::float8 AS nav_ret_1m, nav_ret_ytd::float8 AS nav_ret_ytd,
       nav_ret_1y::float8 AS nav_ret_1y, nav_ret_3y::float8 AS nav_ret_3y
FROM etf_performance WHERE stock_symbol = $1
"#;

/// 前十大持股、行業比重、資產分佈依序排列，各區塊內依來源順序。
const WEIGHTS_SQL: &str = r#"
SELECT kind, as_of, symbol, label, weight
FROM (
    SELECT 'holding' AS kind, 0 AS block, as_of, holding_symbol::text AS symbol, holding_name AS label,
           weight::float8 AS weight, rank::int AS ord
    FROM etf_holding WHERE stock_symbol = $1
    UNION ALL
    SELECT kind::text, CASE kind WHEN 'industry' THEN 1 ELSE 2 END, as_of, '', label,
           weight::float8, sort_order::int
    FROM etf_allocation WHERE stock_symbol = $1
) AS w
ORDER BY block, ord
"#;

const NAVS_SQL: &str = r#"
SELECT date, COALESCE(nav, estimated_nav)::float8 AS nav, market_price::float8 AS market_price,
       premium_rate::float8 AS premium_rate, units_change
FROM etf_daily_nav WHERE stock_symbol = $1
ORDER BY date DESC LIMIT $2
"#;

/// 最近 `$2` 期的前十大持股，由新到舊、期內依排名。
const HOLDING_HISTORY_SQL: &str = r#"
WITH p AS (
    SELECT DISTINCT as_of FROM etf_holding_history WHERE stock_symbol = $1
    ORDER BY as_of DESC LIMIT $2
)
SELECT h.as_of, h.rank, h.holding_symbol::text AS holding_symbol, h.holding_name, h.weight
FROM etf_holding_history AS h
JOIN p USING (as_of)
WHERE h.stock_symbol = $1
ORDER BY h.as_of DESC, h.rank
"#;

/// 由篩選條件組出清單查詢：條件值一律綁定參數，排序欄位只來自白名單。
fn list_query(filter: &EtfListFilter) -> QueryBuilder<Postgres> {
    let mut builder = QueryBuilder::<Postgres>::new(OVERVIEW_COLUMNS);
    builder.push(", COUNT(*) OVER () AS total FROM etf_snapshot AS s WHERE TRUE");
    if !filter.strategies.is_empty() {
        builder
            .push(" AND s.strategy = ANY(")
            .push_bind(filter.strategies.clone())
            .push(")");
    }
    if let Some(region) = &filter.region {
        builder.push(" AND s.region = ").push_bind(region.clone());
    }
    if let Some(frequency) = &filter.frequency {
        builder
            .push(" AND s.dividend_frequency = ")
            .push_bind(frequency.clone());
    }
    if let Some(market_id) = filter.market_id {
        builder.push(" AND s.market_id = ").push_bind(market_id);
    }
    if let Some(keyword) = &filter.keyword {
        // strpos 不把 % 與 _ 當萬用字元。
        builder
            .push(
                " AND strpos(lower(concat_ws(' ', s.stock_symbol, s.name, s.full_name, s.issuer, \
                 s.benchmark, array_to_string(s.themes, ' '))), lower(",
            )
            .push_bind(keyword.clone())
            .push(")) > 0");
    }
    builder
        .push(" ORDER BY s.")
        .push(filter.sort.column())
        .push(if filter.descending { " DESC" } else { " ASC" })
        .push(" NULLS LAST, s.stock_symbol LIMIT ")
        .push_bind(filter.limit);
    builder
}

fn overview(row: &PgRow) -> sqlx::Result<EtfOverview> {
    Ok(EtfOverview {
        stock_symbol: row.try_get("stock_symbol")?,
        name: row.try_get("name")?,
        market_id: row.try_get("market_id")?,
        strategy: row.try_get("strategy")?,
        region: row.try_get("region")?,
        themes: row.try_get("themes")?,
        dividend_frequency: row.try_get("dividend_frequency")?,
        issuer: row.try_get("issuer")?,
        benchmark: row.try_get("benchmark")?,
        quote_date: row.try_get("quote_date")?,
        closing_price: row.try_get("closing_price")?,
        change_range: row.try_get("change_range")?,
        trade_value: row.try_get("trade_value")?,
        nav_date: row.try_get("nav_date")?,
        nav: row.try_get("nav")?,
        premium_rate: row.try_get("premium_rate")?,
        premium_avg_20d: row.try_get("premium_avg_20d")?,
        total_assets: row.try_get("total_assets")?,
        net_flow: row.try_get("net_flow")?,
        dividend_12m: row.try_get("dividend_12m")?,
        yield_12m: row.try_get("yield_12m")?,
        next_ex_date: row.try_get("next_ex_date")?,
        next_dividend: row.try_get("next_dividend")?,
        ret_1m: row.try_get("ret_1m")?,
        ret_ytd: row.try_get("ret_ytd")?,
        ret_1y: row.try_get("ret_1y")?,
        holders: row.try_get("holders")?,
        foreign_net: row.try_get("foreign_net")?,
    })
}

fn profile(row: &PgRow) -> sqlx::Result<EtfProfileView> {
    Ok(EtfProfileView {
        full_name: row.try_get("full_name")?,
        inception_date: row.try_get("inception_date")?,
        is_active: row.try_get("is_active")?,
        management_fee: row.try_get("management_fee")?,
        custodian_fee: row.try_get("custodian_fee")?,
        currency: row.try_get("currency")?,
        source: row.try_get("source")?,
    })
}

fn performance(row: &PgRow) -> sqlx::Result<EtfPerformanceView> {
    Ok(EtfPerformanceView {
        calc_date: row.try_get("calc_date")?,
        ret_1w: row.try_get("ret_1w")?,
        ret_1m: row.try_get("ret_1m")?,
        ret_3m: row.try_get("ret_3m")?,
        ret_6m: row.try_get("ret_6m")?,
        ret_ytd: row.try_get("ret_ytd")?,
        ret_1y: row.try_get("ret_1y")?,
        ret_3y: row.try_get("ret_3y")?,
        ret_5y: row.try_get("ret_5y")?,
        vol_1y: row.try_get("vol_1y")?,
        mdd_1y: row.try_get("mdd_1y")?,
        beta_1y: row.try_get("beta_1y")?,
        sharpe_1y: row.try_get("sharpe_1y")?,
        nav_calc_date: row.try_get("nav_calc_date")?,
        nav_ret_1m: row.try_get("nav_ret_1m")?,
        nav_ret_ytd: row.try_get("nav_ret_ytd")?,
        nav_ret_1y: row.try_get("nav_ret_1y")?,
        nav_ret_3y: row.try_get("nav_ret_3y")?,
    })
}

fn weight(row: &PgRow) -> sqlx::Result<EtfWeightRow> {
    Ok(EtfWeightRow {
        kind: row.try_get("kind")?,
        as_of: row.try_get("as_of")?,
        symbol: row.try_get("symbol")?,
        label: row.try_get("label")?,
        weight: row.try_get("weight")?,
    })
}

fn nav_day(row: &PgRow) -> sqlx::Result<EtfNavDay> {
    Ok(EtfNavDay {
        date: row.try_get("date")?,
        nav: row.try_get("nav")?,
        market_price: row.try_get("market_price")?,
        premium_rate: row.try_get("premium_rate")?,
        units_change: row.try_get("units_change")?,
    })
}

#[async_trait]
impl EtfQueryRepository for PgEtfQueryRepository {
    async fn list_etfs(&self, filter: &EtfListFilter) -> Result<(i64, Vec<EtfOverview>)> {
        let rows = list_query(filter)
            .build()
            .fetch_all(database::get_connection())
            .await
            .context("Failed to list ETFs")?;
        let total = match rows.first() {
            Some(row) => row.try_get("total")?,
            None => 0,
        };
        let items = rows
            .iter()
            .map(overview)
            .collect::<sqlx::Result<Vec<_>>>()?;
        Ok((total, items))
    }

    async fn etf_detail(&self, stock_symbol: &str, nav_days: i64) -> Result<Option<EtfDetail>> {
        let pool = database::get_connection();
        let mut builder = QueryBuilder::<Postgres>::new(OVERVIEW_COLUMNS);
        builder
            .push(" FROM etf_snapshot AS s WHERE s.stock_symbol = ")
            .push_bind(stock_symbol.to_string());
        let Some(row) = builder
            .build()
            .fetch_optional(pool)
            .await
            .context("Failed to fetch ETF overview")?
        else {
            return Ok(None);
        };
        let profile = sqlx::query(PROFILE_SQL)
            .bind(stock_symbol)
            .fetch_optional(pool)
            .await
            .context("Failed to fetch ETF profile")?
            .as_ref()
            .map(profile)
            .transpose()?;
        let performance = sqlx::query(PERFORMANCE_SQL)
            .bind(stock_symbol)
            .fetch_optional(pool)
            .await
            .context("Failed to fetch ETF performance")?
            .as_ref()
            .map(performance)
            .transpose()?;
        let weights = sqlx::query(WEIGHTS_SQL)
            .bind(stock_symbol)
            .fetch_all(pool)
            .await
            .context("Failed to fetch ETF holdings")?
            .iter()
            .map(weight)
            .collect::<sqlx::Result<Vec<_>>>()?;
        let navs = sqlx::query(NAVS_SQL)
            .bind(stock_symbol)
            .bind(nav_days)
            .fetch_all(pool)
            .await
            .context("Failed to fetch ETF navs")?
            .iter()
            .map(nav_day)
            .collect::<sqlx::Result<Vec<_>>>()?;
        Ok(Some(EtfDetail {
            overview: overview(&row)?,
            profile,
            performance,
            weights,
            navs,
        }))
    }

    async fn holding_history(
        &self,
        stock_symbol: &str,
        periods: i64,
    ) -> Result<Option<EtfHoldingHistory>> {
        let pool = database::get_connection();
        let Some(name) = sqlx::query_scalar::<_, String>(
            "SELECT name FROM etf_snapshot WHERE stock_symbol = $1",
        )
        .bind(stock_symbol)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("Failed to fetch ETF name {stock_symbol}"))?
        else {
            return Ok(None);
        };
        let rows: Vec<(
            chrono::NaiveDate,
            i16,
            String,
            String,
            rust_decimal::Decimal,
        )> = sqlx::query_as(HOLDING_HISTORY_SQL)
            .bind(stock_symbol)
            .bind(periods)
            .fetch_all(pool)
            .await
            .with_context(|| format!("Failed to fetch ETF holding history {stock_symbol}"))?;
        let mut history = EtfHoldingHistory {
            stock_symbol: stock_symbol.to_owned(),
            name,
            periods: Vec::new(),
        };
        for (as_of, rank, holding_symbol, holding_name, weight) in rows {
            let record = EtfHoldingRecord {
                rank,
                holding_symbol,
                holding_name,
                weight,
            };
            match history.periods.last_mut() {
                Some(period) if period.as_of == as_of => period.holdings.push(record),
                _ => history.periods.push(EtfHoldingPeriod {
                    as_of,
                    holdings: vec![record],
                }),
            }
        }
        Ok(Some(history))
    }
}

#[cfg(test)]
mod tests;
