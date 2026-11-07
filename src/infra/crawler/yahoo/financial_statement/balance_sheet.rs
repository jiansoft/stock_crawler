//! # Yahoo 資產負債表
//!
//! 對應頁面 `https://tw.stock.yahoo.com/quote/{代號}/balance-sheet`，
//! 資料來源為 `StockServices.balanceSheets`。
//!
//! 只提供單季：資產負債表是時點數字，年度值就是 Q4；而 `period=year` 的回應
//! 還會混入逐日垃圾資料（見 [`super`]），因此不開放其他期別。
//! 共同規則（單位、日期、404）見 [`super`]。

use anyhow::Result;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{FinancialStatement, RawStatement, ReportPeriod, deserialize_decimal, fetch};

/// Yahoo 服務名稱。
const SERVICE: &str = "balanceSheets";

/// 資產負債表項目，金額單位為元（`net_worth` 為每股元）。
///
/// 欄位名稱沿用 Yahoo 原始命名；`short_term_investments` 與 `short_term_investment`
/// 是 Yahoo 回應中兩個不同的欄位（數值不同），不可合併。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSheetItems {
    /// 現金及約當現金。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub cash_and_equivalents: Option<Decimal>,
    /// 短期投資（Yahoo `shortTermInvestments`）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub short_term_investments: Option<Decimal>,
    /// 應收帳款及票據。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub accounts_receivable: Option<Decimal>,
    /// 存貨。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub inventory: Option<Decimal>,
    /// 其他流動資產。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub other_current_assets: Option<Decimal>,
    /// 流動資產。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub current_assets: Option<Decimal>,
    /// 權益法及其他投資（Yahoo `equityAndOtherInvestments`，語意依欄位名推測）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub equity_and_other_investments: Option<Decimal>,
    /// 不動產、廠房及設備。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub property_plant_equipment: Option<Decimal>,
    /// 使用權資產。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub right_of_use_asset: Option<Decimal>,
    /// 非流動資產。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub non_current_assets: Option<Decimal>,
    /// 資產總額。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub total_assets: Option<Decimal>,
    /// 長期投資。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub long_term_investment: Option<Decimal>,
    /// 短期投資（Yahoo `shortTermInvestment`）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub short_term_investment: Option<Decimal>,
    /// 其他資產。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub other_assets: Option<Decimal>,
    /// 短期借款。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub short_term_debt: Option<Decimal>,
    /// 應付短期票券。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub short_term_bills_payable: Option<Decimal>,
    /// 應付帳款及票據。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub accounts_payable: Option<Decimal>,
    /// 一年內到期長期負債。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub current_portion_of_long_term_liabilities: Option<Decimal>,
    /// 流動負債。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub current_liabilities: Option<Decimal>,
    /// 長期負債。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub long_term_liabilities: Option<Decimal>,
    /// 應付公司債。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub bonds_payable: Option<Decimal>,
    /// 其他負債。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub other_liabilities: Option<Decimal>,
    /// 非流動負債。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub non_current_liabilities: Option<Decimal>,
    /// 負債總額。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub total_liabilities: Option<Decimal>,
    /// 股本。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub share_capital: Option<Decimal>,
    /// 保留盈餘。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub retained_earnings: Option<Decimal>,
    /// 權益總額。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub equity: Option<Decimal>,
    /// 每股淨值（元）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub net_worth: Option<Decimal>,
}

/// 單一季度的資產負債表。
pub type BalanceSheet = FinancialStatement<BalanceSheetItems>;

/// `balanceSheets` 的回應外層。
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    list: Vec<RawStatement<BalanceSheetItems>>,
}

/// 取得指定股票的全部單季資產負債表，依期間由新到舊排列。
///
/// 沒有財報的標的（如 ETF）回傳空陣列。
///
/// # Errors
///
/// HTTP 失敗、代號不存在（404，[`super::YahooPageNotFoundError`]）、
/// JSON 格式不符或日期無法對應季別時回傳錯誤。
pub async fn visit(stock_symbol: &str) -> Result<Vec<BalanceSheet>> {
    let response: Response = fetch(SERVICE, stock_symbol, ReportPeriod::Quarter).await?;
    super::parse_statements(response.list, ReportPeriod::Quarter)
}

#[cfg(test)]
mod tests;
