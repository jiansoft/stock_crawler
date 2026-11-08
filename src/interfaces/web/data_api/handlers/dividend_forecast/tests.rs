//! `/portfolio/dividend-forecast` handler：以目前持股預估未來幾個月的現金股利入帳的測試。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::dividend_forecast::{DividendEvent, Holding};

/// 輸入原樣轉入領域預估，依據轉成 snake_case 字串。
#[test]
fn build_response_maps_rows() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
    let inputs = ForecastInputs {
        holdings: vec![Holding {
            stock_symbol: "2330".to_owned(),
            name: "台積電".to_owned(),
            shares: 100,
            closing_price: Some(dec!(1000)),
        }],
        events: vec![DividendEvent {
            stock_symbol: "2330".to_owned(),
            year_of_dividend: 2026,
            quarter: "Q2".to_owned(),
            cash_per_share: dec!(7),
            ex_date: NaiveDate::from_ymd_opt(2026, 12, 10),
            payable_date: NaiveDate::from_ymd_opt(2027, 1, 7),
        }],
        entitlements: Vec::new(),
    };
    let response = build_response(1, &inputs, today, 6);
    assert_eq!((response.member_id, response.total), (1, 700.0));
    assert_eq!(response.items[0].basis, "announced");
    assert_eq!(response.items[0].pay_date, "2027-01-07");
    assert_eq!(response.forward_yield_percent, Some(0.7));
    assert_eq!(response.months.len(), 7);
}

/// 有自身紀錄附 fill_history；沒有時以每股現金股利 ÷ 最新收盤附同組基準；沒有收盤價時兩者皆空。
#[test]
fn attach_fill_hints_prefers_own_history() {
    use crate::domain::dividend::fill::FillSummary;
    use crate::domain::dividend::fill_stats::{FillSample, YieldBucket, bucket_stats};

    let item = |symbol: &str, cash: f64| DividendForecastItem {
        stock_symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        shares: 1_000,
        ex_date: "2026-12-11".to_owned(),
        pay_date: "2027-01-08".to_owned(),
        cash_per_share: cash,
        amount: cash * 1_000.0,
        basis: "announced".to_owned(),
        entitled: false,
        fill_history: None,
        fill_peer: None,
    };
    let mut items = vec![item("2330", 5.0), item("1101", 1.0), item("9999", 1.0)];
    let hints = FillHints::for_test(
        HashMap::from([(
            "2330".to_owned(),
            FillSummary {
                events: 4,
                filled: 4,
                average_trading_days: Some(2.5),
                median_trading_days: Some(2.0),
            },
        )]),
        bucket_stats(&[FillSample {
            bucket: YieldBucket::From3To5,
            trading_days_to_fill: Some(13),
            trading_days_observed: 120,
        }]),
    );
    let prices = HashMap::from([("1101".to_owned(), dec!(25))]);
    attach_fill_hints(&mut items, &hints, &prices);

    assert_eq!(items[0].fill_history.as_ref().map(|h| h.filled), Some(4));
    assert!(items[0].fill_peer.is_none());
    assert_eq!(
        items[1]
            .fill_peer
            .as_ref()
            .map(|p| (p.bucket.as_str(), p.median_trading_days_to_fill)),
        Some(("from_3_to_5", Some(13.0))),
        "1 ÷ 25 = 4%"
    );
    assert!(
        items[2].fill_history.is_none() && items[2].fill_peer.is_none(),
        "沒有收盤價"
    );
}
