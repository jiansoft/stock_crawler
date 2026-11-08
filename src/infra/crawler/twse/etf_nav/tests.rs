//! 證交所 MIS「ETF 預估淨值」的測試。

use rust_decimal_macros::dec;

use super::*;

/// 資料時間轉成帶時區的時間，供測試比對。
fn at(date: NaiveDate, h: u32, m: u32, s: u32) -> chrono::DateTime<FixedOffset> {
    taipei()
        .from_local_datetime(&date.and_hms_opt(h, m, s).expect("時間合法"))
        .single()
        .expect("固定時區不會有歧義")
}

fn fixture() -> Vec<EtfNavRecord> {
    // include_str! 的路徑相對於本檔案（twse/etf_nav/tests.rs）→ twse/testdata/。
    const FIXTURE: &str = include_str!("../testdata/etf_nav_all_etf.json");
    parse(&serde_json::from_str(FIXTURE).expect("fixture 是合法 JSON"))
}

fn find<'a>(records: &'a [EtfNavRecord], symbol: &str) -> &'a EtfNavRecord {
    records
        .iter()
        .find(|r| r.stock_symbol == symbol)
        .expect("fixture 應包含該代號")
}

#[test]
fn parses_every_group_and_skips_empty_ones() {
    let records = fixture();
    assert_eq!(records.len(), 8);
    let day = NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期合法");
    assert!(records.iter().all(|r| r.date == day));
}

#[test]
fn numbers_may_be_json_numbers_or_strings_with_commas() {
    let records = fixture();
    let day = NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期合法");

    let numeric = find(&records, "0050");
    assert_eq!(numeric.outstanding_units, 23_108_000_000);
    assert_eq!(numeric.units_change, 53_000_000);
    assert_eq!(numeric.market_price, Some(dec!(108.15)));
    assert_eq!(numeric.estimated_nav, Some(dec!(108.27)));
    assert_eq!(numeric.premium_rate, Some(dec!(-0.11)));
    assert_eq!(numeric.previous_nav, Some(dec!(107.1100)));
    assert_eq!(numeric.source_time, Some(at(day, 13, 31, 0)));

    let float = find(&records, "00693U");
    assert_eq!(float.outstanding_units, 55_166_000);
    assert_eq!(float.units_change, -500_000);

    let comma = find(&records, "00717");
    assert_eq!(comma.outstanding_units, 116_605_000);
    assert_eq!(comma.premium_rate, Some(dec!(0.70)));

    let text = find(&records, "00703");
    assert_eq!(text.outstanding_units, 9_050_000);
    assert_eq!(text.units_change, -500_000);
    assert_eq!(text.estimated_nav, Some(dec!(18.40)));
}

#[test]
fn dashes_and_blanks_become_none() {
    let records = fixture();
    let missing = find(&records, "00999");
    assert_eq!(missing.market_price, None);
    assert_eq!(missing.estimated_nav, None);
    assert_eq!(missing.premium_rate, None);
    assert_eq!(missing.units_change, 0);
    assert!(missing.previous_nav.is_some());
}

#[test]
fn malformed_bodies_and_rows_are_ignored() {
    assert!(parse(&serde_json::json!({})).is_empty());
    assert!(parse(&serde_json::json!({"a1": "x"})).is_empty());
    let rows = serde_json::json!({"a1": [{"msgArray": [
        {"a": "", "c": 1, "i": "20261021"},
        {"a": "0050", "c": 1, "i": "bad"},
        {"a": "0050", "c": "-", "i": "20261021"},
        {"a": "0050", "c": true, "i": "20261021", "j": "bad"},
        {"a": "0050", "c": 5, "i": "20261021", "e": 0, "f": "abc"}
    ]}]});
    let parsed = parse(&rows);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].market_price, None);
    assert_eq!(parsed[0].estimated_nav, None);
    assert_eq!(parsed[0].source_time, None);
}
