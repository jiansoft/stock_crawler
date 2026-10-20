use rust_decimal_macros::dec;

use super::*;

/// 每個門檻都剛好達標的持股：外資 30%／1,000 張、投信 15%／200 張、連續天數、融資融券、主力。
fn notable() -> HoldingChip {
    HoldingChip {
        stock_symbol: "2330".to_owned(),
        name: "台積電".to_owned(),
        closing_price: Some(dec!(2405)),
        volume: Some(10_000_000),
        foreign_net: Some(-3_000_000),
        trust_net: Some(1_500_000),
        dealer_net: Some(-28_822),
        foreign_streak: Some(-5),
        trust_streak: Some(3),
        margin_previous: Some(5_000),
        margin_balance: Some(5_500),
        short_previous: Some(400),
        short_balance: Some(600),
        broker: Some(HoldingBroker {
            buy_total: 4_145,
            sell_total: 4_045,
            main_share: dec!(-20),
            buyers: Vec::new(),
            sellers: Vec::new(),
        }),
    }
}

#[test]
fn reasons_follow_notification_thresholds() {
    let reasons: Vec<&str> = notable().reasons().iter().map(|r| r.label()).collect();
    assert_eq!(
        reasons,
        [
            "外資大量",
            "投信大量",
            "外資連續",
            "投信連續",
            "融資",
            "融券",
            "主力"
        ]
    );
    let codes: Vec<&str> = notable().reasons().iter().map(|r| r.code()).collect();
    assert_eq!(
        codes,
        [
            "foreign_heavy",
            "trust_heavy",
            "foreign_streak",
            "trust_streak",
            "margin",
            "short",
            "broker"
        ]
    );
}

#[test]
fn reasons_skip_below_thresholds_and_missing_data() {
    let quiet = HoldingChip {
        // 佔量不足、張數不足、天數不足、融資 499 張、融券前日 0 只看張數（199 張不夠）、主力張數 99。
        foreign_net: Some(-2_999_999),
        trust_net: Some(199_999),
        foreign_streak: Some(4),
        trust_streak: Some(-2),
        margin_balance: Some(5_499),
        short_previous: Some(0),
        short_balance: Some(199),
        broker: Some(HoldingBroker {
            buy_total: 99,
            sell_total: 0,
            main_share: dec!(50),
            buyers: Vec::new(),
            sellers: Vec::new(),
        }),
        ..notable()
    };
    assert!(quiet.reasons().is_empty(), "{:?}", quiet.reasons());

    let empty = HoldingChip {
        volume: None,
        foreign_net: None,
        trust_net: None,
        foreign_streak: None,
        trust_streak: None,
        margin_previous: None,
        margin_balance: None,
        short_previous: None,
        short_balance: None,
        broker: None,
        ..notable()
    };
    assert!(empty.reasons().is_empty());
    // 有買賣超但沒有成交量：佔量條件不成立。
    let no_volume = HoldingChip {
        volume: Some(0),
        foreign_streak: None,
        trust_streak: None,
        margin_previous: None,
        short_previous: None,
        broker: None,
        ..notable()
    };
    assert!(no_volume.reasons().is_empty());
}

#[test]
fn helpers_and_broker_net() {
    assert!(heavy_flow(-1_000_000, Some(1_000_000), 100, 1_000));
    assert!(!heavy_flow(1_000_000, None, 1, 1));
    assert!(balance_changed(0, 500, MARGIN_MIN_PERCENT, MARGIN_MIN_LOTS));
    assert!(!balance_changed(
        10_000,
        10_500,
        MARGIN_MIN_PERCENT,
        MARGIN_MIN_LOTS
    ));
    assert!(broker_notable(dec!(-20), -100));
    assert!(!broker_notable(dec!(19.99), 1_000));
    assert_eq!(notable().broker.expect("有主力").main_net(), 100);
}
