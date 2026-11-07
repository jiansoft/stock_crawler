//! # Yahoo 現金流量表
//!
//! 對應頁面 `https://tw.stock.yahoo.com/quote/{代號}/cash-flow-statement`，
//! 資料來源為 `StockServices.cashFlowStatements`，支援單季、累計與年度。
//! 共同規則（單位、日期、404）見 [`super`]。

use anyhow::Result;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{FinancialStatement, RawStatement, ReportPeriod, deserialize_decimal, fetch};

/// Yahoo 服務名稱。
const SERVICE: &str = "cashFlowStatements";

/// 現金流量表項目，金額單位為元。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashFlowItems {
    /// 折舊。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub depreciation: Option<Decimal>,
    /// 攤銷。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub amortization: Option<Decimal>,
    /// 營業活動現金流量。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub operating_cash_flow: Option<Decimal>,
    /// 投資活動現金流量。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub investing_cash_flow: Option<Decimal>,
    /// 籌資活動現金流量。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub financing_cash_flow: Option<Decimal>,
    /// 自由現金流量（營業＋投資）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub free_cash_flow: Option<Decimal>,
    /// 淨現金流量。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub net_cash_flow: Option<Decimal>,
}

/// 單一期別的現金流量表。
pub type CashFlowStatement = FinancialStatement<CashFlowItems>;

/// `cashFlowStatements` 的回應外層。
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    list: Vec<RawStatement<CashFlowItems>>,
}

/// 取得指定股票在該期別的全部現金流量表，依期間由新到舊排列。
///
/// 沒有財報的標的（如 ETF）回傳空陣列。
///
/// # Errors
///
/// HTTP 失敗、代號不存在（404，[`super::YahooPageNotFoundError`]）、
/// JSON 格式不符或日期無法對應期別時回傳錯誤。
pub async fn visit(stock_symbol: &str, period: ReportPeriod) -> Result<Vec<CashFlowStatement>> {
    let response: Response = fetch(SERVICE, stock_symbol, period).await?;
    super::parse_statements(response.list, period)
}

#[cfg(test)]
mod tests;
