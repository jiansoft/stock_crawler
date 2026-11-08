//! 大盤指數防腐層轉換的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_index_acl_mapping() {
    let item = vec![
        "112/05/20".to_string(), // date
        "1,234,567".to_string(), // trading_volume
        "4,567,890".to_string(), // trade_value
        "100,000".to_string(),   // transaction
        "16,000.50".to_string(), // index
        "150.25".to_string(),    // change
    ];

    let cmd = IndexAclMapper::from_strings(&item);
    assert!(cmd.is_some());
    let cmd = cmd.unwrap();
    assert_eq!(cmd.date, NaiveDate::from_ymd_opt(2023, 5, 20).unwrap());
    assert_eq!(cmd.index, dec!(16000.50));
    assert_eq!(cmd.change, dec!(150.25));

    let entity = IndexAclMapper::from_command(&cmd);
    assert_eq!(entity.category, "TAIEX");
    assert_eq!(entity.index, dec!(16000.50));
    assert_eq!(entity.change, dec!(150.25));
}
