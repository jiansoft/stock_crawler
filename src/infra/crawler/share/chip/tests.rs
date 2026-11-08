//! 籌碼資料（三大法人買賣超、融資融券餘額）的共用 DTO 與欄位解析的測試。

use super::*;

#[test]
fn parse_count_handles_separators_and_signs() {
    assert_eq!(parse_count("9,770,520"), Some(9_770_520));
    assert_eq!(parse_count("-2,851,652"), Some(-2_851_652));
    assert_eq!(parse_count(" 30,135 "), Some(30_135));
    assert_eq!(parse_count("--"), None);
    assert_eq!(parse_count(""), None);
}

#[test]
fn institutional_flow_total_sums_three_parties() {
    let flow = InstitutionalFlow {
        foreign: 9_770_520,
        trust: 589_108,
        dealer: 491_655,
    };
    assert_eq!(flow.total(), 10_851_283);
}

#[test]
fn collect_rows_skips_blank_symbols_and_unparsable_rows() {
    let rows = vec![
        vec!["2330".to_string(), "1".to_string()],
        vec![" ".to_string(), "2".to_string()],
        vec!["2317".to_string(), "x".to_string()],
    ];
    let map = collect_rows(&rows, |row| parse_count(&row[1]));
    assert_eq!(map.len(), 1);
    assert_eq!(map["2330"], 1);
}
