//! `/stocks/{symbol}/valuation-band` handler：本益比、股價淨值比在近幾年的相對位置的測試。

use chrono::NaiveDate;
use rust_decimal_macros::dec;

use super::*;

fn row(day: u32, close: Decimal, pe: Decimal, pb: Decimal) -> DailyRatio {
    DailyRatio {
        date: NaiveDate::from_ymd_opt(2026, 4, day).expect("有效日期"),
        close,
        price_earning_ratio: pe,
        price_to_book_ratio: pb,
    }
}

/// 最新一列決定目前值與收盤價，最舊一列是期間起點；兩個倍數各自統計。
#[test]
fn build_response_uses_latest_row_as_current() {
    let rows = [
        row(30, dec!(120), dec!(15), dec!(2)),
        row(29, dec!(100), dec!(10), dec!(0)),
        row(28, dec!(110), dec!(20), dec!(3)),
    ];
    let response = build_response("2330".to_owned(), 5, &rows);
    assert_eq!(response.data_as_of.as_deref(), Some("2026-04-30"));
    assert_eq!(response.from.as_deref(), Some("2026-04-28"));
    assert_eq!(response.closing_price, Some(120.0));

    let pe = response.price_earning_ratio.expect("有本益比");
    assert_eq!((pe.current, pe.median, pe.samples), (Some(15.0), 15.0, 3));
    let implied = pe.implied_prices.expect("有對應股價");
    assert_eq!(implied.median, 120.0);

    let pb = response.price_to_book_ratio.expect("有淨值比");
    assert_eq!((pb.samples, pb.excluded_days), (2, 1));
    assert_eq!(pb.percentile, Some(50.0));
}

/// 沒有日線：日期、收盤價與兩個區間都是 null。
#[test]
fn build_response_without_quotes_is_all_null() {
    let response = build_response("0050".to_owned(), 3, &[]);
    assert_eq!(response.years, 3);
    assert!(response.data_as_of.is_none() && response.from.is_none());
    assert!(response.closing_price.is_none());
    assert!(response.price_earning_ratio.is_none() && response.price_to_book_ratio.is_none());
}
