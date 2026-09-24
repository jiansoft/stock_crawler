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
mod tests {
    use rust_decimal_macros::dec;

    use super::super::FiscalPeriod;
    use super::*;

    fn parse_fixture(json: &str, period: ReportPeriod) -> Vec<CashFlowStatement> {
        let response: Response = serde_json::from_str(json).expect("fixture should parse");
        super::super::parse_statements(response.list, period).expect("statements")
    }

    #[test]
    fn test_parse_quarter_fixture() {
        let result = parse_fixture(
            include_str!("../testdata/financial_cash_flow_quarter_8042.json"),
            ReportPeriod::Quarter,
        );

        assert_eq!(result.len(), 3);
        let latest = &result[0];
        assert_eq!(latest.stock_symbol, "8042");
        assert_eq!(latest.report_period, ReportPeriod::Quarter);
        assert_eq!(
            latest.fiscal_period,
            FiscalPeriod {
                year: 2026,
                quarter: Some(2)
            }
        );
        assert_eq!(latest.items.operating_cash_flow, Some(dec!(-52332000.00)));
        assert_eq!(latest.items.investing_cash_flow, Some(dec!(-176870000.00)));
        assert_eq!(latest.items.financing_cash_flow, Some(dec!(6119000.00)));
        assert_eq!(latest.items.free_cash_flow, Some(dec!(-229202000.00)));
        assert_eq!(latest.items.net_cash_flow, Some(dec!(-193874000.00)));
        assert_eq!(latest.items.depreciation, Some(dec!(51416000.00)));
        assert_eq!(latest.items.amortization, Some(dec!(4129000.00)));
    }

    #[test]
    fn test_parse_year_fixture() {
        let result = parse_fixture(
            include_str!("../testdata/financial_cash_flow_year_8042.json"),
            ReportPeriod::Year,
        );

        assert_eq!(
            result[0].fiscal_period,
            FiscalPeriod {
                year: 2025,
                quarter: None
            }
        );
        assert_eq!(
            result[0].items.operating_cash_flow,
            Some(dec!(313826000.00))
        );
    }

    /// ETF 等沒有財報的標的回空陣列，不是錯誤。
    #[test]
    fn test_parse_empty_list() {
        assert!(parse_fixture(r#"{"list":[]}"#, ReportPeriod::Quarter).is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();

        for period in [
            ReportPeriod::Quarter,
            ReportPeriod::CumulativeQuarter,
            ReportPeriod::Year,
        ] {
            match visit("8042", period).await {
                Ok(result) => {
                    println!("8042 現金流量表 {:?} {} 筆", period, result.len());
                    if let Some(first) = result.first() {
                        println!("{:?}", first);
                    }
                }
                Err(why) => println!("取得 8042 現金流量表失敗: {:?}", why),
            }
        }
    }
}
