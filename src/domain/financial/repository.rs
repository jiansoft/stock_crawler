use crate::{
    core::declare::Quarter,
    domain::financial::{
        entity::{FinancialStatement, HoldingFinancialAlert, HoldingRevenueAlert, MonthlyRevenue},
        statement::{BalanceSheet, CashFlowStatement, IncomeStatement},
    },
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

    // === 持股通知 (Holding alerts) ===

    /// 讀取指定月份**所有持股**的月營收，依年增率由高到低排序。
    ///
    /// `date` 為 yyyyMM 格式的營收月份。只看持股，不看全市場。
    async fn fetch_holding_revenue_alerts(&self, date: i64) -> Result<Vec<HoldingRevenueAlert>>;

    /// 讀取指定年度、季度的**持股**季報，並帶出去年同季的 EPS。
    async fn fetch_holding_financial_alerts(
        &self,
        year: i32,
        quarter: &str,
    ) -> Result<Vec<HoldingFinancialAlert>>;
}

/// 三大財務報表（損益表、資產負債表、現金流量表）之倉儲介面。
///
/// 寫入一律為 upsert：主鍵為 `(stock_symbol, fiscal_year, period_type, quarter, source)`，
/// 重複採集同一期別會覆寫數值。`fetched_at` 每次寫入都更新；`updated_at` 只在數值
/// 真的變動時更新，可用來追蹤來源的事後修正。
#[async_trait]
pub trait FinancialReportRepository: Send + Sync {
    /// 批次 upsert 損益表，回傳受影響列數（含數值未變、僅更新 `fetched_at` 的列）。
    async fn save_income_statements(&self, statements: &[IncomeStatement]) -> Result<u64>;

    /// 批次 upsert 資產負債表，回傳受影響列數（含數值未變、僅更新 `fetched_at` 的列）。
    async fn save_balance_sheets(&self, sheets: &[BalanceSheet]) -> Result<u64>;

    /// 批次 upsert 現金流量表，回傳受影響列數（含數值未變、僅更新 `fetched_at` 的列）。
    async fn save_cash_flow_statements(&self, statements: &[CashFlowStatement]) -> Result<u64>;
}
