//! 報價欄位解析的測試。

use rust_decimal_macros::dec;

use super::*;

// === 報價欄位解析（typed error）測試 ===

/// 驗證「無資料」佔位符（`--`、空白、`N/A`）規則化為 0，屬合法情況。
#[test]
fn parse_quote_decimal_treats_placeholders_as_zero() {
    assert_eq!(parse_quote_decimal("開盤價", "--").unwrap(), Decimal::ZERO);
    assert_eq!(parse_quote_decimal("開盤價", "").unwrap(), Decimal::ZERO);
    assert_eq!(parse_quote_decimal("開盤價", "  ").unwrap(), Decimal::ZERO);
    assert_eq!(
        parse_quote_decimal("開盤價", "----").unwrap(),
        Decimal::ZERO
    );
    assert_eq!(parse_quote_decimal("本益比", "N/A").unwrap(), Decimal::ZERO);
}

/// 驗證千分位逗號與正負號都能正確解析，負數不會被誤判為佔位符。
#[test]
fn parse_quote_decimal_parses_commas_and_signs() {
    assert_eq!(
        parse_quote_decimal("成交股數", "1,234,567").unwrap(),
        dec!(1234567)
    );
    assert_eq!(parse_quote_decimal("漲跌", "-5.00").unwrap(), dec!(-5.00));
    assert_eq!(parse_quote_decimal("漲跌", "+3.5").unwrap(), dec!(3.5));
}

/// 驗證垃圾內容回傳 InvalidDecimal，而不是像舊版一樣默默補 0。
#[test]
fn parse_quote_decimal_rejects_garbage() {
    let err = parse_quote_decimal("收盤價", "abc").unwrap_err();

    match err {
        QuoteParseError::InvalidDecimal { field, raw, .. } => {
            assert_eq!(field, "收盤價");
            assert_eq!(raw, "abc");
        }
        other => panic!("expected InvalidDecimal, got {other:?}"),
    }
}

/// 驗證拒絕比例門檻：0 筆或低於 10% 通過，超過 10% 整批失敗。
/// 除權息當天的漲跌欄標記視為 0（只記 debug），其他非數字仍視為 0。
#[test]
fn soft_quote_decimal_accepts_ex_rights_markers() {
    for raw in ["除權", "除息", " 除權息 "] {
        assert!(is_ex_rights_marker(raw), "{raw}");
        assert_eq!(parse_soft_quote_decimal("漲跌", raw), Decimal::ZERO);
    }
    assert!(!is_ex_rights_marker("X"));
    assert_eq!(parse_soft_quote_decimal("漲跌", "X"), Decimal::ZERO);
}

#[test]
fn rejected_rows_threshold_allows_minor_and_blocks_major() {
    assert!(ensure_rejected_rows_within_threshold("TWSE", 0, 100).is_ok());
    assert!(ensure_rejected_rows_within_threshold("TWSE", 5, 100).is_ok());
    assert!(ensure_rejected_rows_within_threshold("TWSE", 11, 100).is_err());
    // 小樣本也適用：5 列壞 1 列（20%）應整批失敗。
    assert!(ensure_rejected_rows_within_threshold("TPEx", 1, 5).is_err());
}
