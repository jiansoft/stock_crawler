//! 官方本益比與股價淨值比的測試。

use rust_decimal_macros::dec;

use super::*;

/// 正數照常解析（含千分位）；`-`、`N/A`、空字串、0 與負數都是沒有值。
#[test]
fn parse_ratio_handles_placeholders() {
    assert_eq!(parse_ratio(" 13.82 "), Some(dec!(13.82)));
    assert_eq!(parse_ratio("1,234.50"), Some(dec!(1234.50)));
    for raw in ["-", "N/A", "", "0.00", "-3.2"] {
        assert_eq!(parse_ratio(raw), None, "{raw}");
    }
}

/// 兩個倍數都沒有時 has_any 為假。
#[test]
fn has_any_requires_one_ratio() {
    let mut ratio = OfficialRatio {
        stock_symbol: "1101".to_owned(),
        price_earning_ratio: None,
        price_to_book_ratio: None,
    };
    assert!(!ratio.has_any());
    ratio.price_to_book_ratio = Some(dec!(1.2));
    assert!(ratio.has_any());
}
