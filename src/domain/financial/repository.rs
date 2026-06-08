use crate::{
    core::declare::Quarter,
    domain::financial::entity::{FinancialStatement, MonthlyRevenue},
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 財報與營收領域之倉儲介面 (Repository Trait)。
///
/// 隔離資料庫存取細節，並定義與財務報表、月營收與價格估值相關之讀寫合約。
#[async_trait]
pub trait FinancialRepository: Send + Sync {
    // === 財務報表 (FinancialStatement) ===

    /// 儲存或更新單筆財務報表實體。
    async fn save_financial_statement(&self, statement: &FinancialStatement) -> Result<()>;

    /// 批次儲存或更新多筆財務報表實體。
    async fn batch_save_financial_statements(
        &self,
        statements: &[FinancialStatement],
    ) -> Result<()>;

    /// 僅新增或更新財務報表的每股盈餘 (EPS) 欄位。
    async fn save_earnings_per_share(&self, statement: &FinancialStatement) -> Result<()>;

    /// 補寫年度匯總 EPS (quarter = "")。
    async fn save_annual_eps(&self, statement: &FinancialStatement) -> Result<()>;

    /// 更新既有財報實體的 ROE 與 ROA。
    async fn update_statement_roe_roa(&self, statement: &FinancialStatement) -> Result<()>;

    /// 取得指定年度的年度財報。
    async fn fetch_annual_statements(&self, year: i32) -> Result<Vec<FinancialStatement>>;

    /// 取得季度財報中 ROE、ROA 或每股淨值為零的數據。
    async fn fetch_roe_or_roa_equal_to_zero(
        &self,
        year: Option<i32>,
        quarter: Option<Quarter>,
    ) -> Result<Vec<FinancialStatement>>;

    /// 取得指定年度（包含回溯 10 年內）缺少年報的股票與年份清單（回傳的實體僅包含 `security_code` 與 `year`）。
    async fn fetch_without_annual_statements(&self, year: i32) -> Result<Vec<FinancialStatement>>;

    /// 取得指定年度、指定季別集合的 EPS 累計。
    async fn fetch_cumulative_eps(
        &self,
        security_code: &str,
        year: i32,
        quarters: Vec<Quarter>,
    ) -> Result<Decimal>;

    // === 月營收 (MonthlyRevenue) ===

    /// 新增或更新單月營收實體。
    async fn save_monthly_revenue(&self, revenue: &MonthlyRevenue) -> Result<()>;

    /// 讀取最近兩個月的營收實體清單。
    async fn fetch_last_two_months_revenues(&self) -> Result<Vec<MonthlyRevenue>>;

    /// 重建最新營收日期索引表 (revenue_last_date)。
    async fn rebuild_revenue_last_date(&self) -> Result<()>;

    // === 價格估值 (PriceEstimate) ===

    /// 依指定日期與年份區間，批次重建所有個股價格估值。
    async fn rebuild_price_estimates(&self, date: NaiveDate, years: String) -> Result<()>;
}
