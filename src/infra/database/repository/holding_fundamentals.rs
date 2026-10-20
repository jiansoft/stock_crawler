//! 持股基本面（最新月營收與最近一季三率）的 PostgreSQL 查詢。
//!
//! `/portfolio/summary` 與每週市場摘要共用；新鮮度與產業排除規則見
//! [`crate::domain::portfolio::health::HoldingFundamentalsQuery`]。

use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;

use super::portfolio::PgPortfolioRepository;
use crate::domain::portfolio::health::{
    FundamentalSignals, HoldingFundamentals, HoldingFundamentalsQuery, MARGIN_EXCLUDED_INDUSTRY,
};
use crate::infra::database;

/// `$1` 代號陣列、`$2` 不取三率的產業。
const HOLDING_FUNDAMENTALS_SQL: &str = r#"
SELECT held.stock_symbol,
    rv.revenue_month, rv.revenue_yoy, rv.revenue_decline_months,
    m.margin_year, m.margin_quarter, m.gross_margin, m.operating_margin, m.net_margin,
    m.last_year_gross_margin, m.last_year_operating_margin, m.last_year_net_margin
FROM UNNEST($1::text[]) AS held(stock_symbol)
LEFT JOIN stocks s ON s.stock_symbol = held.stock_symbol
LEFT JOIN LATERAL (
    -- 最近 12 個月營收：最新一個月的年增率，與從最新一個月起連續年減（月份不中斷）的月數。
    SELECT
        MAX(revenue_month) FILTER (WHERE rn = 1) AS revenue_month,
        MAX(yoy) FILTER (WHERE rn = 1) AS revenue_yoy,
        COALESCE(
            MIN(rn) FILTER (WHERE NOT (yoy < 0 AND ord = first_ord - rn + 1)) - 1,
            COUNT(*)
        )::smallint AS revenue_decline_months
    FROM (
        SELECT revenue_month, yoy, ord,
            ROW_NUMBER() OVER (ORDER BY ord DESC) AS rn,
            FIRST_VALUE(ord) OVER (ORDER BY ord DESC) AS first_ord
        FROM (
            SELECT "Date" AS revenue_month,
                "ComparedWithLastYearSameMonth" AS yoy,
                ("Date" / 100)::int * 12 + ("Date" % 100)::int AS ord
            FROM "Revenue"
            WHERE "SecurityCode" = held.stock_symbol
            ORDER BY "Date" DESC
            LIMIT 12
        ) recent
    ) ranked
    HAVING MAX(ord) >= EXTRACT(YEAR FROM CURRENT_DATE)::int * 12
        + EXTRACT(MONTH FROM CURRENT_DATE)::int - 2
) rv ON TRUE
LEFT JOIN LATERAL (
    -- Yahoo 最新一季單季損益表的三率與去年同季的三率（營收為 0 時為 NULL）；`$2` 產業不取。
    SELECT cur.fiscal_year AS margin_year,
        SUBSTRING(cur.quarter FROM 2)::smallint AS margin_quarter,
        cur.gross_profit::float8 / NULLIF(cur.revenue, 0) * 100 AS gross_margin,
        cur.operating_profit::float8 / NULLIF(cur.revenue, 0) * 100 AS operating_margin,
        cur.net_income::float8 / NULLIF(cur.revenue, 0) * 100 AS net_margin,
        prev.gross_profit::float8 / NULLIF(prev.revenue, 0) * 100 AS last_year_gross_margin,
        prev.operating_profit::float8 / NULLIF(prev.revenue, 0) * 100 AS last_year_operating_margin,
        prev.net_income::float8 / NULLIF(prev.revenue, 0) * 100 AS last_year_net_margin
    FROM (
        SELECT fiscal_year, quarter, revenue, gross_profit, operating_profit, net_income
        FROM income_statement
        WHERE source = 'yahoo'
          AND stock_symbol = held.stock_symbol
          AND period_type = 'single'
          AND s.stock_industry_id IS DISTINCT FROM $2
        ORDER BY fiscal_year DESC, quarter DESC
        LIMIT 1
    ) cur
    LEFT JOIN income_statement prev
           ON prev.source = 'yahoo'
          AND prev.stock_symbol = held.stock_symbol
          AND prev.period_type = 'single'
          AND prev.fiscal_year = cur.fiscal_year - 1
          AND prev.quarter = cur.quarter
    WHERE (EXTRACT(YEAR FROM CURRENT_DATE)::int * 4 + EXTRACT(QUARTER FROM CURRENT_DATE)::int)
        - (cur.fiscal_year * 4 + SUBSTRING(cur.quarter FROM 2)::int) BETWEEN 0 AND 2
) m ON TRUE
ORDER BY held.stock_symbol
"#;

/// [`HOLDING_FUNDAMENTALS_SQL`] 的一列。
#[derive(Debug, Default, sqlx::FromRow)]
pub(crate) struct FundamentalsRow {
    stock_symbol: String,
    revenue_month: Option<i64>,
    revenue_yoy: Option<Decimal>,
    revenue_decline_months: Option<i16>,
    margin_year: Option<i32>,
    margin_quarter: Option<i16>,
    gross_margin: Option<f64>,
    operating_margin: Option<f64>,
    net_margin: Option<f64>,
    last_year_gross_margin: Option<f64>,
    last_year_operating_margin: Option<f64>,
    last_year_net_margin: Option<f64>,
}

impl From<FundamentalsRow> for HoldingFundamentals {
    /// 三率對去年同季的變化在這裡算；任一邊查不到時為 `None`。
    fn from(row: FundamentalsRow) -> Self {
        let change = |current: Option<f64>, last_year: Option<f64>| Some(current? - last_year?);
        Self {
            revenue_month: row.revenue_month,
            revenue_yoy: row.revenue_yoy,
            margin_period: row.margin_year.zip(row.margin_quarter),
            margins: [row.gross_margin, row.operating_margin, row.net_margin],
            signals: FundamentalSignals {
                revenue_decline_months: row
                    .revenue_decline_months
                    .and_then(|months| u8::try_from(months).ok()),
                operating_margin: row.operating_margin,
                margin_changes: [
                    change(row.gross_margin, row.last_year_gross_margin),
                    change(row.operating_margin, row.last_year_operating_margin),
                    change(row.net_margin, row.last_year_net_margin),
                ],
            },
            stock_symbol: row.stock_symbol,
        }
    }
}

#[async_trait]
impl HoldingFundamentalsQuery for PgPortfolioRepository {
    async fn fetch_holding_fundamentals(
        &self,
        symbols: &[String],
    ) -> Result<Vec<HoldingFundamentals>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<FundamentalsRow> = sqlx::query_as(HOLDING_FUNDAMENTALS_SQL)
            .bind(symbols)
            .bind(MARGIN_EXCLUDED_INDUSTRY as i32)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch holding fundamentals")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三率變化只在本季與去年同季都有值時才算；連續年減月數超出 u8 時視為查不到。
    #[test]
    fn row_converts_to_domain_fundamentals() {
        let fundamentals = HoldingFundamentals::from(FundamentalsRow {
            stock_symbol: "2330".to_owned(),
            revenue_month: Some(202609),
            revenue_yoy: Some(Decimal::from(-5)),
            revenue_decline_months: Some(3),
            margin_year: Some(2026),
            margin_quarter: Some(2),
            gross_margin: Some(40.0),
            operating_margin: Some(-1.5),
            net_margin: Some(2.0),
            last_year_gross_margin: Some(45.0),
            last_year_operating_margin: None,
            last_year_net_margin: Some(4.0),
        });
        assert_eq!(fundamentals.margin_period, Some((2026, 2)));
        assert_eq!(fundamentals.signals.revenue_decline_months, Some(3));
        assert_eq!(
            fundamentals.signals.margin_changes,
            [Some(-5.0), None, Some(-2.0)]
        );
        let overflow = HoldingFundamentals::from(FundamentalsRow {
            revenue_decline_months: Some(-1),
            ..Default::default()
        });
        assert_eq!(overflow.signals.revenue_decline_months, None);
    }

    /// 沒有代號時不查資料庫。
    #[tokio::test]
    async fn empty_symbols_skip_the_query() {
        let fundamentals = PgPortfolioRepository::new()
            .fetch_holding_fundamentals(&[])
            .await
            .expect("空清單");
        assert!(fundamentals.is_empty());
    }
}
