//! 其他資料防腐層轉換的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_qfii_to_update_command() {
    let dto = QfiiDto {
        stock_symbol: "2330".to_string(),
        issued_share: 100000,
        shares_held: 50000,
        share_holding_percentage: dec!(75.5),
    };

    let cmd = QfiiAclMapper::from_qfii(&dto);
    assert_eq!(cmd.symbol, "2330");
    assert_eq!(cmd.shares_held, 50000);
    assert_eq!(cmd.share_holding_percentage, dec!(75.5));
    assert_eq!(cmd.issued_share, 100000);
}

#[test]
fn test_stock_weight_acl_mapping() {
    let dto = crate::infra::crawler::taifex::stock_weight::StockWeight {
        rank: 1,
        stock_symbol: "2330".to_string(),
        weight: dec!(28.5),
    };

    let cmd = StockWeightAclMapper::from_dto(&dto);
    assert_eq!(cmd.symbol, "2330");
    assert_eq!(cmd.weight, dec!(28.5));
}
