use crate::{
    core::declare::Quarter,
    domain::financial::{
        entity::{
            FinancialStatement as DomainFinancialStatement, MonthlyRevenue as DomainMonthlyRevenue,
        },
        repository::FinancialRepository,
    },
    infra::database::table::{
        estimate::Estimate as TableEstimate,
        financial_statement::{self, FinancialStatement as TableFinancialStatement},
        revenue::{self, Revenue as TableRevenue},
    },
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// PostgreSQL 實作之財報與營收倉儲。
///
/// 基於 PostgreSQL (SQLx) 實現 `FinancialRepository` 介面，並處理資料庫 Table 模型與領域實體之間的映射。
#[derive(Default)]
pub struct PgFinancialRepository;

impl PgFinancialRepository {
    /// 建立 `PgFinancialRepository` 新實例。
    pub fn new() -> Self {
        PgFinancialRepository
    }
}

// === 實體映射實作 ===

impl From<DomainFinancialStatement> for TableFinancialStatement {
    fn from(domain: DomainFinancialStatement) -> Self {
        // 將領域實體映射至資料庫 Table 模型，保留所有屬性對齊。
        TableFinancialStatement {
            serial: domain.serial,
            security_code: domain.security_code,
            year: domain.year,
            quarter: domain.quarter,
            gross_profit: domain.gross_profit,
            operating_profit_margin: domain.operating_profit_margin,
            pre_tax_income: domain.pre_tax_income,
            net_income: domain.net_income,
            net_asset_value_per_share: domain.net_asset_value_per_share,
            sales_per_share: domain.sales_per_share,
            earnings_per_share: domain.earnings_per_share,
            profit_before_tax: domain.profit_before_tax,
            return_on_equity: domain.return_on_equity,
            return_on_assets: domain.return_on_assets,
            created_time: domain.created_time,
            updated_time: domain.updated_time,
        }
    }
}

impl From<TableFinancialStatement> for DomainFinancialStatement {
    fn from(table: TableFinancialStatement) -> Self {
        // 將資料庫 Table 模型轉換為領域層實體。
        DomainFinancialStatement {
            serial: table.serial,
            security_code: table.security_code,
            year: table.year,
            quarter: table.quarter,
            gross_profit: table.gross_profit,
            operating_profit_margin: table.operating_profit_margin,
            pre_tax_income: table.pre_tax_income,
            net_income: table.net_income,
            net_asset_value_per_share: table.net_asset_value_per_share,
            sales_per_share: table.sales_per_share,
            earnings_per_share: table.earnings_per_share,
            profit_before_tax: table.profit_before_tax,
            return_on_equity: table.return_on_equity,
            return_on_assets: table.return_on_assets,
            created_time: table.created_time,
            updated_time: table.updated_time,
        }
    }
}

impl From<DomainMonthlyRevenue> for TableRevenue {
    fn from(domain: DomainMonthlyRevenue) -> Self {
        // 將領域層營收映射至資料庫 Table 模型。
        TableRevenue {
            stock_symbol: domain.stock_symbol,
            monthly: domain.monthly,
            last_month: domain.last_month,
            last_year_this_month: domain.last_year_this_month,
            monthly_accumulated: domain.monthly_accumulated,
            last_year_monthly_accumulated: domain.last_year_monthly_accumulated,
            compared_with_last_month: domain.compared_with_last_month,
            compared_with_last_year_same_month: domain.compared_with_last_year_same_month,
            accumulated_compared_with_last_year: domain.accumulated_compared_with_last_year,
            avg_price: domain.avg_price,
            lowest_price: domain.lowest_price,
            highest_price: domain.highest_price,
            date: domain.date,
            create_time: domain.create_time,
        }
    }
}

impl From<TableRevenue> for DomainMonthlyRevenue {
    fn from(table: TableRevenue) -> Self {
        // 將資料庫 Table 模型營收轉換為領域層營收實體。
        DomainMonthlyRevenue {
            stock_symbol: table.stock_symbol,
            monthly: table.monthly,
            last_month: table.last_month,
            last_year_this_month: table.last_year_this_month,
            monthly_accumulated: table.monthly_accumulated,
            last_year_monthly_accumulated: table.last_year_monthly_accumulated,
            compared_with_last_month: table.compared_with_last_month,
            compared_with_last_year_same_month: table.compared_with_last_year_same_month,
            accumulated_compared_with_last_year: table.accumulated_compared_with_last_year,
            avg_price: table.avg_price,
            lowest_price: table.lowest_price,
            highest_price: table.highest_price,
            date: table.date,
            create_time: table.create_time,
        }
    }
}

#[async_trait]
impl FinancialRepository for PgFinancialRepository {
    // === 財務報表 (FinancialStatement) ===

    async fn save_financial_statement(&self, statement: &DomainFinancialStatement) -> Result<()> {
        // 轉換為 Table 實體並呼叫 Table 層的 upsert。
        let table_entity = TableFinancialStatement::from(statement.clone());
        table_entity.upsert().await?;
        Ok(())
    }

    async fn batch_save_financial_statements(
        &self,
        statements: &[DomainFinancialStatement],
    ) -> Result<()> {
        // 將所有領域實體批次轉換為 Table 實體。
        let table_entities: Vec<TableFinancialStatement> = statements
            .iter()
            .map(|s| TableFinancialStatement::from(s.clone()))
            .collect();
        // 呼叫 Table 層的批次寫入。
        TableFinancialStatement::batch_upsert(&table_entities).await?;
        Ok(())
    }

    async fn save_earnings_per_share(&self, statement: &DomainFinancialStatement) -> Result<()> {
        // 僅新增或更新每股盈餘欄位。
        let table_entity = TableFinancialStatement::from(statement.clone());
        table_entity.upsert_earnings_per_share().await?;
        Ok(())
    }

    async fn save_annual_eps(&self, statement: &DomainFinancialStatement) -> Result<()> {
        // 補寫年度匯總 EPS。
        let table_entity = TableFinancialStatement::from(statement.clone());
        table_entity.upsert_annual_eps().await?;
        Ok(())
    }

    async fn update_statement_roe_roa(&self, statement: &DomainFinancialStatement) -> Result<()> {
        // 更新 ROE 與 ROA。
        let table_entity = TableFinancialStatement::from(statement.clone());
        table_entity.update_roe_roa().await?;
        Ok(())
    }

    async fn fetch_annual_statements(&self, year: i32) -> Result<Vec<DomainFinancialStatement>> {
        // 讀取 Table 年度財報並轉為領域實體清單。
        let table_statements = financial_statement::fetch_annual(year).await?;
        let domain_statements = table_statements
            .into_iter()
            .map(DomainFinancialStatement::from)
            .collect();
        Ok(domain_statements)
    }

    async fn fetch_roe_or_roa_equal_to_zero(
        &self,
        year: Option<i32>,
        quarter: Option<Quarter>,
    ) -> Result<Vec<DomainFinancialStatement>> {
        // 讀取 ROE、ROA 為零的資料並轉為領域實體清單。
        let table_statements =
            financial_statement::fetch_roe_or_roa_equal_to_zero(year, quarter).await?;
        let domain_statements = table_statements
            .into_iter()
            .map(DomainFinancialStatement::from)
            .collect();
        Ok(domain_statements)
    }

    async fn fetch_without_annual_statements(
        &self,
        year: i32,
    ) -> Result<Vec<DomainFinancialStatement>> {
        // 讀取缺少年報的股票與年份清單。
        let table_statements = financial_statement::fetch_without_annual(year).await?;
        let domain_statements = table_statements
            .into_iter()
            .map(DomainFinancialStatement::from)
            .collect();
        Ok(domain_statements)
    }

    async fn fetch_cumulative_eps(
        &self,
        security_code: &str,
        year: i32,
        quarters: Vec<Quarter>,
    ) -> Result<Decimal> {
        // 讀取指定季度的 EPS 累計金額。
        financial_statement::fetch_cumulative_eps(security_code, year, quarters).await
    }

    // === 月營收 (MonthlyRevenue) ===

    async fn save_monthly_revenue(&self, revenue: &DomainMonthlyRevenue) -> Result<()> {
        // 轉為 Table 實體並呼叫 Table 層的 upsert。
        let table_entity = TableRevenue::from(revenue.clone());
        table_entity.upsert().await?;
        Ok(())
    }

    async fn fetch_last_two_months_revenues(&self) -> Result<Vec<DomainMonthlyRevenue>> {
        // 讀取最近兩個月營收並轉為領域實體。
        let table_revenues = revenue::fetch_last_two_month().await?;
        let domain_revenues = table_revenues
            .into_iter()
            .map(DomainMonthlyRevenue::from)
            .collect();
        Ok(domain_revenues)
    }

    async fn rebuild_revenue_last_date(&self) -> Result<()> {
        // 重建最新營收日期索引。
        revenue::rebuild_revenue_last_date().await?;
        Ok(())
    }

    // === 價格估值 (PriceEstimate) ===

    async fn rebuild_price_estimates(&self, date: NaiveDate, years: String) -> Result<()> {
        // 依據指定條件批次重建價格估值。
        TableEstimate::upsert_all(date, years).await?;
        Ok(())
    }
}
