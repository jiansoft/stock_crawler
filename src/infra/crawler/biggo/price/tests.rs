//! BigGo 即時報價的測試；連外的測試標 #[ignore]。

use super::*;
use crate::infra::crawler::biggo::Envelope;

fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn parse(json: &str) -> CurrentPrice {
    let envelope: Envelope<CurrentPrice> =
        serde_json::from_str(json).expect("fixture should parse");
    envelope.data.expect("fixture should have data")
}

/// 2330 在 2026-10-05 09:48 的報價：當天的快照可以採用。
#[test]
fn validate_accepts_a_quote_from_today() {
    let data = parse(include_str!("../testdata/current_price_2330.json"));
    let quote = validate("2330", data, day(2026, 10, 5)).unwrap();
    assert_eq!(
        quote,
        Quote {
            price: 2575.0,
            change: 75.0,
            change_range: 3.0,
        }
    );
}

/// 1423 當天還沒成交時 BigGo 回 10-02 的快照，不可當成即時價。
#[test]
fn validate_rejects_a_quote_from_another_day() {
    let data = parse(include_str!("../testdata/current_price_1423.json"));
    let error = validate("1423", data, day(2026, 10, 5)).unwrap_err();
    assert!(error.to_string().contains("2026-10-02"), "{error}");

    let data = parse(include_str!("../testdata/current_price_2330.json"));
    assert!(validate("2330", data, day(2026, 10, 6)).is_err());
}

#[test]
fn validate_rejects_missing_fields() {
    let today = day(2026, 10, 5);
    let base = || CurrentPrice {
        current_price: Some(100.0),
        daily_change: Some(1.0),
        daily_change_percent: Some(1.0),
        snapshot_date: Some(1_791_164_883),
    };
    assert!(validate("2330", base(), today).is_ok());
    for broken in [
        CurrentPrice {
            current_price: None,
            ..base()
        },
        CurrentPrice {
            current_price: Some(0.0),
            ..base()
        },
        CurrentPrice {
            snapshot_date: None,
            ..base()
        },
        CurrentPrice {
            daily_change: None,
            ..base()
        },
        CurrentPrice {
            daily_change_percent: None,
            ..base()
        },
    ] {
        assert!(validate("2330", broken, today).is_err());
    }
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行（盤中才有當天報價）"]
async fn test_get_stock_price() {
    dotenvy::dotenv().ok();
    crate::infra::crawler::log_stock_price_test::<BigGo>("2330").await;
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行（盤中才有當天報價）"]
async fn test_get_stock_quotes() {
    dotenvy::dotenv().ok();
    let quotes = BigGo::get_stock_quotes("6488")
        .await
        .expect("BigGo quotes during market hours");
    println!("{quotes:?}");
    assert!(quotes.price > 0.0);
}

/// 台北時間 00:00 是 UTC 前一天 16:00。
#[test]
fn taipei_date_uses_utc_plus_eight() {
    // 2026-10-04 16:00:00 UTC = 2026-10-05 00:00:00 台北
    assert_eq!(taipei_date(1_791_129_600), Some(day(2026, 10, 5)));
    assert_eq!(taipei_date(1_791_129_599), Some(day(2026, 10, 4)));
}
