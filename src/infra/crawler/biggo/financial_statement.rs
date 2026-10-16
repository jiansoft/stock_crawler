//! # BigGo 三大財報
//!
//! Yahoo 三大財報失敗（例如 HTTP 500）時的備援來源。每檔股票共 9 次請求：
//!
//! | 報表 | endpoint | 期別 |
//! |------|----------|------|
//! | 損益表 | `/stock/statements/profit`＋`/stock/statements/eps` | 單季、年度 |
//! | 現金流量表 | `/stock/statements/cash-flow` | 單季、年度 |
//! | 資產負債表 | `/stock/statements/total-assets`＋`liabilities`＋`equity` | 單季 |
//!
//! ## 回應規則（2026-10 實測，2330／2881／1101／8069／2603／5880 逐期比對）
//!
//! - 金額單位為**元**（JSON 整數），EPS 為元（JSON 小數）；缺值為 `null`，不可當成 0。
//! - `period` 單季為 `"2026 Q2"`，年度為 `"2026"`。
//! - **年度資料含當年度累計**：年中查詢時 `"2026"` 是今年至今的累計（2330 的 2026 EPS
//!   49.33 只是前兩季），不是全年數字，呼叫端要用 [`retain_completed_years`] 濾掉。
//! - 只取與 Yahoo 欄位逐期吻合的項目；應收帳款、短期借款、保留盈餘等定義不同的欄位不取。
//! - 金融業（金控、銀行）的資產負債表與營業利益多為 `null`，照實保留。
//!
//! 子模組：[`rows`]（BigGo 回應的資料列與讀取）、[`merge`]（期別解析與各 endpoint 依期別合併）。

use std::time::Duration;

use anyhow::Result;
use rust_decimal::Decimal;

use self::merge::{merge_balance, merge_income, parse_cash_flow};
use self::rows::{
    AssetsRow, CashFlowRow, EpsRow, EquityRow, LiabilitiesRow, ProfitRow, fetch_list, fetch_typed,
};

mod merge;
mod rows;

/// 同一檔股票的請求之間停頓的時間。
const REQUEST_PAUSE: Duration = Duration::from_millis(400);

/// 報表期別（對應 BigGo 的 `type` 參數）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    /// 單季（`single`）。
    Single,
    /// 年度（`annual`）；當年度為今年至今的累計。
    Annual,
}

impl ReportKind {
    /// BigGo API 使用的 `type` 參數值。
    fn as_param(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Annual => "annual",
        }
    }
}

/// 財報所屬期間。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FiscalPeriod {
    /// 西元年度。
    pub year: i32,
    /// 季別 1～4；年度報表為 `None`。
    pub quarter: Option<u8>,
}

/// 單一期別的財務報表，`T` 為各報表的項目。
#[derive(Debug, Clone, PartialEq)]
pub struct Statement<T> {
    /// 股票代號（不含 `.TW` 後綴）。
    pub stock_symbol: String,
    /// 報表期別。
    pub kind: ReportKind,
    /// 所屬期間。
    pub period: FiscalPeriod,
    /// 報表項目。
    pub items: T,
}

/// 損益表項目（金額單位：元）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IncomeItems {
    /// 營業收入。
    pub revenue: Option<i64>,
    /// 營業毛利。
    pub gross_profit: Option<i64>,
    /// 營業費用。
    pub operating_expenses: Option<i64>,
    /// 營業利益。
    pub operating_income: Option<i64>,
    /// 稅後淨利（含非控制權益，對應 Yahoo 的 `netIncome`）。
    pub income_after_taxes: Option<i64>,
    /// 每股盈餘（元）。
    pub eps: Option<Decimal>,
}

/// 資產負債表項目（金額單位：元）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BalanceItems {
    /// 現金及約當現金。
    pub cash_and_cash_equivalents: Option<i64>,
    /// 透過損益按公允價值衡量之金融資產－流動（對應 Yahoo 的 `shortTermInvestments`）。
    pub current_financial_assets_fvtpl: Option<i64>,
    /// 存貨。
    pub inventories: Option<i64>,
    /// 其他流動資產。
    pub other_current_assets: Option<i64>,
    /// 流動資產。
    pub current_assets: Option<i64>,
    /// 採權益法之投資。
    pub investment_accounted_equity_method: Option<i64>,
    /// 不動產、廠房及設備。
    pub property_plant_equipment: Option<i64>,
    /// 使用權資產。
    pub right_of_use_asset: Option<i64>,
    /// 非流動資產。
    pub noncurrent_assets: Option<i64>,
    /// 資產總額。
    pub total_assets: Option<i64>,
    /// 流動負債。
    pub current_liabilities: Option<i64>,
    /// 應付公司債。
    pub bonds_payable: Option<i64>,
    /// 非流動負債。
    pub noncurrent_liabilities: Option<i64>,
    /// 負債總額。
    pub total_liabilities: Option<i64>,
    /// 股本。
    pub capital_stock: Option<i64>,
    /// 權益總額。
    pub equity: Option<i64>,
}

/// 現金流量表項目（金額單位：元）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CashFlowItems {
    /// 營業活動現金流量。
    pub operating_cash_flow: Option<i64>,
    /// 投資活動現金流量。
    pub investing_cash_flow: Option<i64>,
    /// 籌資活動現金流量。
    pub financing_cash_flow: Option<i64>,
    /// 自由現金流量。
    pub free_cash_flow: Option<i64>,
    /// 本期現金淨增減。
    pub net_cash_flow: Option<i64>,
}

/// 抓取損益表（損益與 EPS 兩支 endpoint 依期別合併）。
///
/// # Errors
///
/// 任一請求失敗、回應格式不符或期別無法解析時回傳錯誤。
pub async fn income_statements(
    stock_symbol: &str,
    kind: ReportKind,
) -> Result<Vec<Statement<IncomeItems>>> {
    let profit: Vec<ProfitRow> =
        fetch_typed("/stock/statements/profit", stock_symbol, kind).await?;
    tokio::time::sleep(REQUEST_PAUSE).await;
    let eps: Vec<EpsRow> = fetch_typed("/stock/statements/eps", stock_symbol, kind).await?;
    merge_income(stock_symbol, kind, profit, eps)
}

/// 抓取現金流量表。
///
/// # Errors
///
/// 請求失敗、回應格式不符或期別無法解析時回傳錯誤。
pub async fn cash_flow_statements(
    stock_symbol: &str,
    kind: ReportKind,
) -> Result<Vec<Statement<CashFlowItems>>> {
    let rows: Vec<CashFlowRow> =
        fetch_typed("/stock/statements/cash-flow", stock_symbol, kind).await?;
    parse_cash_flow(stock_symbol, kind, rows)
}

/// 抓取單季資產負債表（資產、負債、權益三支 endpoint 依期別合併）。
///
/// # Errors
///
/// 任一請求失敗、回應格式不符或期別無法解析時回傳錯誤。
pub async fn balance_sheets(stock_symbol: &str) -> Result<Vec<Statement<BalanceItems>>> {
    let assets: Vec<AssetsRow> = fetch_list("/stock/statements/total-assets", stock_symbol).await?;
    tokio::time::sleep(REQUEST_PAUSE).await;
    let liabilities: Vec<LiabilitiesRow> =
        fetch_list("/stock/statements/liabilities", stock_symbol).await?;
    tokio::time::sleep(REQUEST_PAUSE).await;
    let equity: Vec<EquityRow> = fetch_list("/stock/statements/equity", stock_symbol).await?;
    merge_balance(stock_symbol, assets, liabilities, equity)
}

/// 只保留已有第四季單季資料的年度。
///
/// BigGo 的年度資料包含當年度至今的累計，必須確定該年第四季已公布，年度數字才是全年。
pub fn retain_completed_years<T>(
    annual: Vec<Statement<T>>,
    singles: &[FiscalPeriod],
) -> Vec<Statement<T>> {
    annual
        .into_iter()
        .filter(|statement| {
            singles.contains(&FiscalPeriod {
                year: statement.period.year,
                quarter: Some(4),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
