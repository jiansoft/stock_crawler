//! 持股近期除權息回應：金額換算、合計、填息自身紀錄優先於同組基準、沒有事件時不附填息。

use std::collections::HashMap;

use rust_decimal_macros::dec;

use super::*;
use crate::domain::dividend::fill::FillSummary;
use crate::domain::dividend::fill_stats::{BucketStats, YieldBucket};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).expect("合法日期")
}

fn dividend(symbol: &str, cash: Decimal, stock: Decimal) -> UpcomingDividend {
    UpcomingDividend {
        ex_date: day(10),
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        cash_dividend: cash,
        stock_dividend: stock,
        closing_price: dec!(100),
        shares: 1999,
    }
}

/// 金額依股數換算並合計；有自身紀錄附 fill_history，沒有時依殖利率組附 fill_peer。
#[test]
fn build_response_maps_amounts_and_fill_hints() {
    let peer = BucketStats {
        bucket: YieldBucket::of(dec!(4), dec!(100)).expect("3～5% 組"),
        events: 10,
        mature_events: 8,
        fill_rate_20: Some(50.0),
        fill_rate_60: Some(70.0),
        fill_rate_120: Some(80.0),
        median_trading_days: Some(15.0),
    };
    let hints = FillHints::for_test(
        HashMap::from([(
            "2330".to_owned(),
            FillSummary {
                events: 5,
                filled: 4,
                average_trading_days: Some(3.0),
                median_trading_days: Some(2.0),
            },
        )]),
        vec![peer],
    );
    let dividends = [
        dividend("2330", dec!(5), Decimal::ZERO),
        dividend("2412", dec!(4), dec!(0.55)),
    ];
    let response = build_response(7, day(2), day(15), &dividends, Some(&hints));
    assert_eq!(
        (
            response.member_id,
            response.from.as_str(),
            response.to.as_str()
        ),
        (7, "2026-11-02", "2026-11-15")
    );
    assert_eq!(response.total_cash_amount, 17991.0, "1999 × 5 + 1999 × 4");
    let [first, second] = response.items.as_slice() else {
        panic!("應有兩筆：{:?}", response.items);
    };
    assert_eq!((first.cash_amount, first.stock_shares), (9995.0, 0));
    assert!(first.fill_history.is_some(), "有自身紀錄附 fill_history");
    assert!(first.fill_peer.is_none(), "有自身紀錄就不附同組基準");
    assert_eq!((second.stock_dividend, second.stock_shares), (0.55, 109));
    assert!(second.fill_history.is_none());
    assert_eq!(
        second
            .fill_peer
            .as_ref()
            .and_then(|peer| peer.median_trading_days_to_fill),
        Some(15.0)
    );
}

/// 沒有事件時合計為 0、清單為空。
#[test]
fn build_response_without_events_is_empty() {
    let response = build_response(0, day(2), day(15), &[], None);
    assert_eq!(response.total_cash_amount, 0.0);
    assert!(response.items.is_empty());
}
