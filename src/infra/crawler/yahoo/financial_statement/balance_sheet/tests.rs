//! Yahoo 資產負債表的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::super::FiscalPeriod;
use super::*;

#[test]
fn test_parse_quarter_fixture() {
    let response: Response = serde_json::from_str(include_str!(
        "../../testdata/financial_balance_sheet_quarter_8042.json"
    ))
    .expect("fixture should parse");
    let result =
        super::super::parse_statements(response.list, ReportPeriod::Quarter).expect("rows");

    assert_eq!(result.len(), 2);
    let latest = &result[0];
    assert_eq!(latest.stock_symbol, "8042");
    assert_eq!(
        latest.fiscal_period,
        FiscalPeriod {
            year: 2026,
            quarter: Some(2)
        }
    );
    let items = &latest.items;
    assert_eq!(items.cash_and_equivalents, Some(dec!(1570007000.00)));
    // 沒有小數點的字串也要能解析。
    assert_eq!(items.short_term_investments, Some(dec!(150932000)));
    assert_eq!(items.short_term_investment, Some(dec!(390217000.00)));
    assert_eq!(items.total_assets, Some(dec!(10137998000.00)));
    assert_eq!(items.total_liabilities, Some(dec!(5493780000.00)));
    assert_eq!(items.equity, Some(dec!(4644218000.00)));
    assert_eq!(items.net_worth, Some(dec!(34.64)));
    assert_eq!(items.short_term_bills_payable, Some(dec!(0.00)));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();

    match visit("8042").await {
        Ok(result) => {
            println!("8042 資產負債表 {} 筆", result.len());
            if let Some(first) = result.first() {
                println!("{:?}", first);
            }
        }
        Err(why) => println!("取得 8042 資產負債表失敗: {:?}", why),
    }
}
