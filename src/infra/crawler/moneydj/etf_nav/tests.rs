//! MoneyDJ ETF 歷史淨值與市價的測試。

use rust_decimal_macros::dec;

use super::*;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("日期應合法")
}

#[test]
fn parses_fixture() {
    let points = parse(include_str!("../testdata/etf_nav_0050.txt")).expect("fixture 應可解析");
    assert_eq!(points.len(), 14);
    assert_eq!(
        points.first(),
        Some(&EtfNavHistoryPoint {
            date: day(10, 1),
            nav: dec!(112.8700),
            market_price: Some(dec!(112.9000)),
        })
    );
    assert_eq!(
        points.last(),
        Some(&EtfNavHistoryPoint {
            date: day(10, 21),
            nav: dec!(108.2700),
            market_price: Some(dec!(108.1500)),
        })
    );
}

#[test]
fn url_uses_tw_suffix_and_compact_dates() {
    assert_eq!(
        url("00679B", day(10, 1), day(10, 21)),
        "https://www.moneydj.com/ETF/X/xdjbcd/Basic0003BCD.xdjbcd?etfid=00679B.TW&b=20261001&c=20261021"
    );
}

#[test]
fn empty_body_means_unknown_symbol() {
    assert!(parse("").expect("空字串").is_empty());
    assert!(parse("\0\n").expect("NUL").is_empty());
}

#[test]
fn skips_bad_nav_and_keeps_missing_price() {
    let points = parse("20261020,20261021,x 0,108.27,1 107.2,0,5").expect("應可解析");
    assert_eq!(
        points,
        vec![EtfNavHistoryPoint {
            date: day(10, 21),
            nav: dec!(108.27),
            market_price: None,
        }]
    );
}

#[test]
fn rejects_changed_format() {
    assert!(parse("20261020,20261021 1,2").is_err());
    assert!(parse("20261020,20261021 1,2 3").is_err());
}
