//! 月營收防腐層轉換的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_revenue_acl_mapping() {
    let dto = RevenueDto {
        stock_symbol: "2330".to_string(),
        monthly: dec!(1000.0),
        last_month: dec!(900.0),
        last_year_this_month: dec!(950.0),
        monthly_accumulated: dec!(5000.0),
        last_year_monthly_accumulated: dec!(4500.0),
        compared_with_last_month: dec!(11.1),
        compared_with_last_year_same_month: dec!(5.3),
        accumulated_compared_with_last_year: dec!(11.1),
        date: 202605,
    };

    let cmd = RevenueAclMapper::from_dto(&dto);
    assert_eq!(cmd.symbol, "2330");
    assert_eq!(cmd.monthly, dec!(1000.0));
    assert_eq!(cmd.date, 202605);

    let entity = RevenueAclMapper::from_command(&cmd);
    assert_eq!(entity.stock_symbol, "2330");
    assert_eq!(entity.monthly, dec!(1000.0));
    assert_eq!(entity.date, 202605);
}
