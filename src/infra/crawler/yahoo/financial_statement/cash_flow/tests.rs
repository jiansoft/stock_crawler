//! Yahoo 現金流量表的測試；連外的測試標 #[ignore]。

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
        include_str!("../../testdata/financial_cash_flow_quarter_8042.json"),
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
        include_str!("../../testdata/financial_cash_flow_year_8042.json"),
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
