//! 股利防腐層轉換的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_yahoo_dividend_acl_mapping() {
    let detail = crate::infra::crawler::yahoo::dividend::YahooDividendDetail {
        year: 2025,
        year_of_dividend: 2024,
        quarter: "Q4".to_string(),
        cash_dividend: dec!(3.5),
        stock_dividend: dec!(0.2),
        ex_dividend_date1: "2025-07-01".to_string(),
        ex_dividend_date2: "-".to_string(),
        payable_date1: "2025-08-01".to_string(),
        payable_date2: "-".to_string(),
    };

    let cmd = YahooDividendAclMapper::from_dto("2454", &detail);
    assert_eq!(cmd.security_code, "2454");
    assert_eq!(cmd.sum, dec!(3.7));

    let entity = YahooDividendAclMapper::from_command(&cmd);
    assert_eq!(entity.security_code, "2454");
    assert_eq!(entity.sum, dec!(3.7));
    assert_eq!(entity.ex_dividend_date_cash, "2025-07-01");
}
