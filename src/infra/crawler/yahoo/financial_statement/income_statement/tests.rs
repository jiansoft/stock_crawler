//! Yahoo 損益表的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::super::FiscalPeriod;
use super::*;

fn parse_fixture(json: &str, period: ReportPeriod) -> Vec<IncomeStatement> {
    let response: Response = serde_json::from_str(json).expect("fixture should parse");
    super::super::parse_statements(response.list, period).expect("statements")
}

#[test]
fn test_parse_cumulative_quarter_fixture() {
    let result = parse_fixture(
        include_str!("../../testdata/financial_income_statement_quarter_sum_8042.json"),
        ReportPeriod::CumulativeQuarter,
    );

    assert_eq!(result.len(), 2);
    let latest = &result[0];
    assert_eq!(latest.stock_symbol, "8042");
    assert_eq!(latest.report_period, ReportPeriod::CumulativeQuarter);
    assert_eq!(
        latest.fiscal_period,
        FiscalPeriod {
            year: 2026,
            quarter: Some(2)
        }
    );
    let items = &latest.items;
    assert_eq!(items.revenue, Some(dec!(2380422000.00)));
    assert_eq!(items.gross_profit, Some(dec!(505926000.00)));
    assert_eq!(items.operating_profit, Some(dec!(184249000.00)));
    assert_eq!(items.non_operating_income, Some(dec!(315963000.00)));
    assert_eq!(items.net_income, Some(dec!(381320000.00)));
    assert_eq!(items.owner_parent_profit, Some(dec!(372410000.00)));
    assert_eq!(items.eps, Some(dec!(2.84)));
    assert_eq!(items.bps, Some(dec!(34.64)));
    assert_eq!(items.net_income_acc_4q, Some(dec!(502.45)));
}

#[test]
fn test_parse_year_fixture() {
    let result = parse_fixture(
        include_str!("../../testdata/financial_income_statement_year_8042.json"),
        ReportPeriod::Year,
    );

    let latest = &result[0];
    assert_eq!(
        latest.fiscal_period,
        FiscalPeriod {
            year: 2025,
            quarter: None
        }
    );
    assert_eq!(latest.items.revenue, Some(dec!(3765883000.00)));
    assert_eq!(latest.items.eps, Some(dec!(0.89)));
    // 年度與單季的 netIncomeAcc4Q 是 null，必須是 None 而不是 0。
    assert_eq!(latest.items.net_income_acc_4q, None);
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
                println!("8042 損益表 {:?} {} 筆", period, result.len());
                if let Some(first) = result.first() {
                    println!("{:?}", first);
                }
            }
            Err(why) => println!("取得 8042 損益表失敗: {:?}", why),
        }
    }
}
