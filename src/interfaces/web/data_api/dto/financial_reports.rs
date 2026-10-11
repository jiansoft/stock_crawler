//! `/stocks/{symbol}/financial-reports` 的 request、response 與 OpenAPI schema。
//!
//! 一次回傳單一股票的三大財務報表：損益表、資產負債表與現金流量表。資料來自
//! `income_statement`、`balance_sheet`、`cash_flow_statement` 三張表（06:00 Yahoo
//! 財報排程寫入，Yahoo 失敗時以 BigGo 備援）。金額單位一律是新台幣千元，每股
//! 數值單位是新台幣元；來源沒提供的欄位為 `null`，不補成 0。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 金額欄位的單位，回應內固定帶出，呼叫端不必另查文件。
pub(crate) const AMOUNT_UNIT: &str = "thousand_twd";

/// OpenAPI 文件使用的三大報表期間類型。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum ReportPeriodTypeValue {
    /// 單季資料。
    Quarterly,
    /// 年度資料；資產負債表為時點數值，改回傳各年第 4 季季末。
    Annual,
}

/// 三大報表 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct FinancialReportParams {
    /// 期間類型：`quarterly`（預設，單季）或 `annual`（年度）。
    #[param(value_type = ReportPeriodTypeValue, default = "quarterly")]
    pub(crate) period_type: Option<String>,
    /// 每張報表最多回傳期數，預設 8，範圍 1–40。
    #[param(minimum = 1, maximum = 40, default = 8)]
    pub(crate) limit: Option<u16>,
}

/// 單期損益表。
///
/// 金額單位為新台幣千元、可為負數；年度與單季 EPS 皆為來源官方值，年度 EPS
/// 不等於四季相加。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct IncomeStatementItem {
    /// 財報所屬年度（西元）。
    pub(crate) fiscal_year: i32,
    /// 期間標記：單季為 `Q1`–`Q4`，年度為 `A`。
    pub(crate) quarter: String,
    /// 資料來源：`yahoo` 或 `biggo`（同一期兩者都有時取 `yahoo`）。
    pub(crate) source: String,
    /// 營業收入。
    pub(crate) revenue: Option<i64>,
    /// 營業毛利。
    pub(crate) gross_profit: Option<i64>,
    /// 推銷費用；金融業多為 0。
    pub(crate) selling_expenses: Option<i64>,
    /// 管理費用；金融業多為 0。
    pub(crate) admin_expenses: Option<i64>,
    /// 研究發展費用；金融業多為 0。
    pub(crate) rd_expenses: Option<i64>,
    /// 營業費用。
    pub(crate) operating_expenses: Option<i64>,
    /// 營業利益。
    pub(crate) operating_profit: Option<i64>,
    /// 營業外收入及支出。
    pub(crate) non_operating_income: Option<i64>,
    /// 稅前淨利。
    pub(crate) profit_before_tax: Option<i64>,
    /// 本期稅後淨利（含非控制權益）。
    pub(crate) net_income: Option<i64>,
    /// 歸屬母公司業主淨利。
    pub(crate) owner_parent_profit: Option<i64>,
    /// 每股營收（元）。
    pub(crate) revenue_per_share: Option<f64>,
    /// 每股營業利益（元）。
    pub(crate) operating_profit_per_share: Option<f64>,
    /// 每股稅前淨利（元）。
    pub(crate) profit_before_tax_per_share: Option<f64>,
    /// 每股盈餘（元）。
    pub(crate) eps: Option<f64>,
    /// 期末每股淨值（元）；時點數值，不可跨期相加。
    pub(crate) bps: Option<f64>,
    /// 資料最近一次變更時間（UTC ISO 8601）。
    pub(crate) updated_at: String,
}

/// 單季季末資產負債表。
///
/// 金額單位為新台幣千元；金融業的流動／非流動分類欄位來源多為 0。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BalanceSheetItem {
    /// 財報所屬年度（西元）。
    pub(crate) fiscal_year: i32,
    /// 季別 `Q1`–`Q4`（季末時點）。
    pub(crate) quarter: String,
    /// 資料來源：`yahoo` 或 `biggo`（同一期兩者都有時取 `yahoo`）。
    pub(crate) source: String,
    /// 現金及約當現金。
    pub(crate) cash_and_equivalents: Option<i64>,
    /// 短期投資（來源欄位 `shortTermInvestments`，與 `short_term_investment` 是不同欄位）。
    pub(crate) short_term_investments: Option<i64>,
    /// 應收帳款及票據。
    pub(crate) accounts_receivable: Option<i64>,
    /// 存貨。
    pub(crate) inventory: Option<i64>,
    /// 其他流動資產。
    pub(crate) other_current_assets: Option<i64>,
    /// 流動資產。
    pub(crate) current_assets: Option<i64>,
    /// 權益法及其他投資。
    pub(crate) equity_and_other_investments: Option<i64>,
    /// 不動產、廠房及設備。
    pub(crate) property_plant_equipment: Option<i64>,
    /// 使用權資產。
    pub(crate) right_of_use_asset: Option<i64>,
    /// 非流動資產。
    pub(crate) non_current_assets: Option<i64>,
    /// 資產總額。
    pub(crate) total_assets: Option<i64>,
    /// 長期投資。
    pub(crate) long_term_investment: Option<i64>,
    /// 短期投資（來源欄位 `shortTermInvestment`，與 `short_term_investments` 是不同欄位）。
    pub(crate) short_term_investment: Option<i64>,
    /// 其他資產。
    pub(crate) other_assets: Option<i64>,
    /// 短期借款。
    pub(crate) short_term_debt: Option<i64>,
    /// 應付短期票券。
    pub(crate) short_term_bills_payable: Option<i64>,
    /// 應付帳款及票據。
    pub(crate) accounts_payable: Option<i64>,
    /// 一年內到期長期負債。
    pub(crate) current_portion_of_long_term_liabilities: Option<i64>,
    /// 流動負債。
    pub(crate) current_liabilities: Option<i64>,
    /// 長期負債。
    pub(crate) long_term_liabilities: Option<i64>,
    /// 應付公司債。
    pub(crate) bonds_payable: Option<i64>,
    /// 其他負債。
    pub(crate) other_liabilities: Option<i64>,
    /// 非流動負債。
    pub(crate) non_current_liabilities: Option<i64>,
    /// 負債總額。
    pub(crate) total_liabilities: Option<i64>,
    /// 股本。
    pub(crate) share_capital: Option<i64>,
    /// 保留盈餘，可為負數。
    pub(crate) retained_earnings: Option<i64>,
    /// 權益總額，可為負數。
    pub(crate) equity: Option<i64>,
    /// 每股淨值（元），可為負數。
    pub(crate) book_value_per_share: Option<f64>,
    /// 資料最近一次變更時間（UTC ISO 8601）。
    pub(crate) updated_at: String,
}

/// 單期現金流量表。
///
/// 金額單位為新台幣千元、可為負數。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct CashFlowStatementItem {
    /// 財報所屬年度（西元）。
    pub(crate) fiscal_year: i32,
    /// 期間標記：單季為 `Q1`–`Q4`，年度為 `A`。
    pub(crate) quarter: String,
    /// 資料來源：`yahoo` 或 `biggo`（同一期兩者都有時取 `yahoo`）。
    pub(crate) source: String,
    /// 折舊費用。
    pub(crate) depreciation: Option<i64>,
    /// 攤銷費用。
    pub(crate) amortization: Option<i64>,
    /// 營業活動現金流量。
    pub(crate) operating_cash_flow: Option<i64>,
    /// 投資活動現金流量。
    pub(crate) investing_cash_flow: Option<i64>,
    /// 融資活動現金流量。
    pub(crate) financing_cash_flow: Option<i64>,
    /// 自由現金流（來源定義：營業現金流加投資現金流）。
    pub(crate) free_cash_flow: Option<i64>,
    /// 現金及約當現金淨增減；不等於上面三項現金流相加。
    pub(crate) net_cash_flow: Option<i64>,
    /// 資料最近一次變更時間（UTC ISO 8601）。
    pub(crate) updated_at: String,
}

/// 三大報表的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FinancialReportResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 實際使用的期間類型：`quarterly` 或 `annual`。
    pub(crate) period_type: String,
    /// 金額欄位單位，固定為 `thousand_twd`（新台幣千元）；每股欄位單位為元。
    pub(crate) amount_unit: &'static str,
    /// 最新一期損益表的期間（`YYYY-Qn` 或 `YYYY-A`）；沒有損益表時依序改看
    /// 現金流量表、資產負債表，三者皆空時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 損益表，依期間由新到舊。
    pub(crate) income_statements: Vec<IncomeStatementItem>,
    /// 資產負債表，依期間由新到舊；`annual` 時為各年第 4 季季末。
    pub(crate) balance_sheets: Vec<BalanceSheetItem>,
    /// 現金流量表，依期間由新到舊。
    pub(crate) cash_flow_statements: Vec<CashFlowStatementItem>,
}
