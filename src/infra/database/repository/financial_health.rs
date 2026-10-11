//! 財務體質（F 分數與選股用季報指標）的 PostgreSQL 倉儲實作。
//!
//! 讀取 `income_statement`、`balance_sheet`、`cash_flow_statement` 全市場的數值，
//! 寫入 `financial_health`。三張表的主鍵都含 `source`，同一期 Yahoo 與 BigGo 並存時
//! 以 `DISTINCT ON` 只留 Yahoo。

use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::domain::financial::{
    health::{AnnualFigures, FinancialHealthSnapshot, QuarterFigures, YearFigures},
    repository::FinancialHealthRepository,
};
use crate::infra::database;

/// 每檔每年一列：三張表各自去重後，以「股票＋年度」聯集再左接，任一張表有資料的年度都會出現。
const ANNUAL_FIGURES_SQL: &str = r#"
WITH inc AS (
    SELECT DISTINCT ON (stock_symbol, fiscal_year)
        stock_symbol, fiscal_year, TRUE AS present, revenue, gross_profit, net_income
    FROM income_statement
    WHERE period_type = 'annual' AND fiscal_year >= $1
    ORDER BY stock_symbol, fiscal_year, (source = 'yahoo') DESC, source
), cf AS (
    SELECT DISTINCT ON (stock_symbol, fiscal_year)
        stock_symbol, fiscal_year, operating_cash_flow
    FROM cash_flow_statement
    WHERE period_type = 'annual' AND fiscal_year >= $1
    ORDER BY stock_symbol, fiscal_year, (source = 'yahoo') DESC, source
), bs AS (
    SELECT DISTINCT ON (stock_symbol, fiscal_year)
        stock_symbol, fiscal_year, total_assets, current_assets, current_liabilities,
        non_current_liabilities, share_capital
    FROM balance_sheet
    WHERE period_type = 'single' AND quarter = 'Q4' AND fiscal_year >= $1
    ORDER BY stock_symbol, fiscal_year, (source = 'yahoo') DESC, source
), keys AS (
    SELECT stock_symbol, fiscal_year FROM inc
    UNION SELECT stock_symbol, fiscal_year FROM cf
    UNION SELECT stock_symbol, fiscal_year FROM bs
)
SELECT keys.stock_symbol, keys.fiscal_year, COALESCE(inc.present, FALSE) AS has_income,
    inc.revenue, inc.gross_profit, inc.net_income, cf.operating_cash_flow,
    bs.total_assets, bs.current_assets, bs.current_liabilities, bs.non_current_liabilities,
    bs.share_capital
FROM keys
LEFT JOIN inc USING (stock_symbol, fiscal_year)
LEFT JOIN cf USING (stock_symbol, fiscal_year)
LEFT JOIN bs USING (stock_symbol, fiscal_year)
"#;

/// 每檔每季一列，組法同 [`ANNUAL_FIGURES_SQL`]。
const QUARTER_FIGURES_SQL: &str = r#"
WITH inc AS (
    SELECT DISTINCT ON (stock_symbol, fiscal_year, quarter)
        stock_symbol, fiscal_year, quarter, TRUE AS present, revenue, gross_profit
    FROM income_statement
    WHERE period_type = 'single' AND fiscal_year >= $1
    ORDER BY stock_symbol, fiscal_year, quarter, (source = 'yahoo') DESC, source
), bs AS (
    SELECT DISTINCT ON (stock_symbol, fiscal_year, quarter)
        stock_symbol, fiscal_year, quarter, total_assets, total_liabilities
    FROM balance_sheet
    WHERE period_type = 'single' AND fiscal_year >= $1
    ORDER BY stock_symbol, fiscal_year, quarter, (source = 'yahoo') DESC, source
), cf AS (
    SELECT DISTINCT ON (stock_symbol, fiscal_year, quarter)
        stock_symbol, fiscal_year, quarter, free_cash_flow
    FROM cash_flow_statement
    WHERE period_type = 'single' AND fiscal_year >= $1
    ORDER BY stock_symbol, fiscal_year, quarter, (source = 'yahoo') DESC, source
), keys AS (
    SELECT stock_symbol, fiscal_year, quarter FROM inc
    UNION SELECT stock_symbol, fiscal_year, quarter FROM bs
    UNION SELECT stock_symbol, fiscal_year, quarter FROM cf
)
SELECT keys.stock_symbol, keys.fiscal_year, SUBSTRING(keys.quarter FROM 2)::smallint AS quarter,
    COALESCE(inc.present, FALSE) AS has_income, inc.revenue, inc.gross_profit,
    bs.total_assets, bs.total_liabilities, cf.free_cash_flow
FROM keys
LEFT JOIN inc USING (stock_symbol, fiscal_year, quarter)
LEFT JOIN bs USING (stock_symbol, fiscal_year, quarter)
LEFT JOIN cf USING (stock_symbol, fiscal_year, quarter)
"#;

#[derive(sqlx::FromRow)]
struct AnnualRow {
    stock_symbol: String,
    fiscal_year: i32,
    has_income: bool,
    revenue: Option<i64>,
    gross_profit: Option<i64>,
    net_income: Option<i64>,
    operating_cash_flow: Option<i64>,
    total_assets: Option<i64>,
    current_assets: Option<i64>,
    current_liabilities: Option<i64>,
    non_current_liabilities: Option<i64>,
    share_capital: Option<i64>,
}

impl From<AnnualRow> for (String, i32, AnnualFigures) {
    fn from(row: AnnualRow) -> Self {
        let figures = YearFigures {
            revenue: row.revenue,
            gross_profit: row.gross_profit,
            net_income: row.net_income,
            operating_cash_flow: row.operating_cash_flow,
            total_assets: row.total_assets,
            current_assets: row.current_assets,
            current_liabilities: row.current_liabilities,
            non_current_liabilities: row.non_current_liabilities,
            share_capital: row.share_capital,
        };
        (
            row.stock_symbol,
            row.fiscal_year,
            AnnualFigures {
                has_income: row.has_income,
                figures,
            },
        )
    }
}

#[derive(sqlx::FromRow)]
struct QuarterRow {
    stock_symbol: String,
    fiscal_year: i32,
    quarter: i16,
    has_income: bool,
    revenue: Option<i64>,
    gross_profit: Option<i64>,
    total_assets: Option<i64>,
    total_liabilities: Option<i64>,
    free_cash_flow: Option<i64>,
}

impl From<QuarterRow> for (String, QuarterFigures) {
    fn from(row: QuarterRow) -> Self {
        let figures = QuarterFigures {
            fiscal_year: row.fiscal_year,
            // 資料表的 CHECK 限制季別只有 Q1–Q4。
            quarter: u8::try_from(row.quarter).unwrap_or_default(),
            has_income: row.has_income,
            revenue: row.revenue,
            gross_profit: row.gross_profit,
            total_assets: row.total_assets,
            total_liabilities: row.total_liabilities,
            free_cash_flow: row.free_cash_flow,
        };
        (row.stock_symbol, figures)
    }
}

/// 對應資料表 `financial_health`。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgFinancialHealthRepository;

impl PgFinancialHealthRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FinancialHealthRepository for PgFinancialHealthRepository {
    async fn fetch_annual_figures(
        &self,
        since_year: i32,
    ) -> Result<Vec<(String, i32, AnnualFigures)>> {
        let rows: Vec<AnnualRow> = sqlx::query_as(ANNUAL_FIGURES_SQL)
            .bind(since_year)
            .fetch_all(database::get_connection())
            .await
            .with_context(|| format!("讀取財務體質年報數值失敗: since_year={since_year}"))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn fetch_quarter_figures(
        &self,
        since_year: i32,
    ) -> Result<Vec<(String, QuarterFigures)>> {
        let rows: Vec<QuarterRow> = sqlx::query_as(QUARTER_FIGURES_SQL)
            .bind(since_year)
            .fetch_all(database::get_connection())
            .await
            .with_context(|| format!("讀取財務體質季報數值失敗: since_year={since_year}"))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn replace_financial_health(&self, snapshots: &[FinancialHealthSnapshot]) -> Result<u64> {
        // 以欄為單位展開成陣列，用一個 UNNEST 寫入整批（約 2,000 列）。
        let symbols: Vec<&str> = snapshots.iter().map(|s| s.stock_symbol.as_str()).collect();
        let f_score_years: Vec<Option<i32>> = snapshots.iter().map(|s| s.f_score_year).collect();
        let f_scores: Vec<Option<i16>> =
            snapshots.iter().map(|s| s.f_score.map(i16::from)).collect();
        let evaluated: Vec<i16> = snapshots
            .iter()
            .map(|s| i16::from(s.f_score_evaluated))
            .collect();
        let quarter_years: Vec<Option<i32>> = snapshots.iter().map(|s| s.quarter_year).collect();
        let quarters: Vec<Option<i16>> =
            snapshots.iter().map(|s| s.quarter.map(i16::from)).collect();
        let gross_margins: Vec<Option<Decimal>> =
            snapshots.iter().map(|s| s.gross_margin_percent).collect();
        let debt_ratios: Vec<Option<Decimal>> =
            snapshots.iter().map(|s| s.debt_ratio_percent).collect();
        let free_cash_flows: Vec<Option<i64>> =
            snapshots.iter().map(|s| s.free_cash_flow_ttm).collect();

        let mut tx = database::get_tx().await?;
        sqlx::query("DELETE FROM financial_health")
            .execute(&mut *tx)
            .await
            .context("清除 financial_health 失敗")?;
        let inserted = sqlx::query(
            r#"INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated, quarter_year, quarter, gross_margin_percent, debt_ratio_percent, free_cash_flow_ttm)
SELECT * FROM UNNEST($1::varchar[], $2::int[], $3::smallint[], $4::smallint[], $5::int[], $6::smallint[], $7::numeric[], $8::numeric[], $9::bigint[])"#,
        )
        .bind(&symbols)
        .bind(&f_score_years)
        .bind(&f_scores)
        .bind(&evaluated)
        .bind(&quarter_years)
        .bind(&quarters)
        .bind(&gross_margins)
        .bind(&debt_ratios)
        .bind(&free_cash_flows)
        .execute(&mut *tx)
        .await
        .context("寫入 financial_health 失敗")?
        .rows_affected();
        tx.commit().await?;

        Ok(inserted)
    }
}

#[cfg(test)]
mod tests;
