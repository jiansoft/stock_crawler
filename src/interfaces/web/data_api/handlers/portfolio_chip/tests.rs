use chrono::NaiveDate;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::chip::HoldingBroker;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("固定日期合法")
}

fn chip() -> HoldingChip {
    HoldingChip {
        stock_symbol: "2330".to_owned(),
        name: "台積電".to_owned(),
        closing_price: Some(dec!(2405)),
        volume: Some(21_402_430),
        foreign_net: Some(-9_364_622),
        trust_net: Some(1_029_673),
        dealer_net: Some(-28_822),
        foreign_streak: Some(-10),
        trust_streak: Some(15),
        margin_previous: Some(34_111),
        margin_balance: Some(34_401),
        short_previous: Some(9),
        short_balance: Some(14),
        broker: Some(HoldingBroker {
            buy_total: 4_145,
            sell_total: 12_057,
            main_share: dec!(-36.95),
            buyers: vec![BrokerNetRecord {
                name: "凱基-台北".to_owned(),
                buy: 900,
                sell: 100,
                net: 800,
                share: dec!(3.74),
            }],
            sellers: Vec::new(),
        }),
    }
}

/// 股換算成張、佔量與增減、主力與原因代碼；沒有資料的持股全為 null、原因為空。
#[test]
fn response_converts_lots_and_reasons() {
    let empty = HoldingChip {
        stock_symbol: "1101".to_owned(),
        name: String::new(),
        closing_price: None,
        volume: None,
        foreign_net: None,
        trust_net: None,
        dealer_net: None,
        foreign_streak: None,
        trust_streak: None,
        margin_previous: None,
        margin_balance: None,
        short_previous: None,
        short_balance: None,
        broker: None,
    };
    let day = ChipTradingDay {
        date: day(20),
        previous: Some(day(19)),
        next: None,
    };
    let body = response(Some(day), vec![chip(), empty]);
    assert_eq!(body.date.as_deref(), Some("2026-10-20"));
    assert_eq!(body.previous_date.as_deref(), Some("2026-10-19"));
    assert!(body.next_date.is_none());
    assert_eq!(body.notable, 1);

    let tsmc = &body.items[0];
    assert_eq!(
        (
            tsmc.foreign_net,
            tsmc.trust_net,
            tsmc.dealer_net,
            tsmc.volume
        ),
        (Some(-9_365), Some(1_030), Some(-29), Some(21_402))
    );
    assert_eq!(
        (tsmc.foreign_share, tsmc.trust_share),
        (Some(-43.75), Some(4.81))
    );
    assert_eq!(
        (tsmc.margin_change, tsmc.short_change),
        (Some(290), Some(5))
    );
    assert_eq!((tsmc.foreign_streak, tsmc.trust_streak), (-10, 15));
    assert_eq!(tsmc.closing_price, Some(2405.0));
    let broker = tsmc.broker.as_ref().expect("有主力");
    assert_eq!((broker.main_net, broker.main_share), (-7_912, -36.95));
    assert_eq!(broker.buyers[0].share, 3.74);
    assert_eq!(
        tsmc.reasons,
        ["foreign_heavy", "foreign_streak", "trust_streak", "broker"]
    );

    let quiet = &body.items[1];
    assert!(quiet.reasons.is_empty() && quiet.broker.is_none());
    assert!(quiet.foreign_share.is_none() && quiet.margin_change.is_none());
    assert_eq!(quiet.foreign_streak, 0);

    let none = response(None, Vec::new());
    assert!(none.date.is_none() && none.items.is_empty() && none.notable == 0);
}

#[test]
fn share_of_volume_needs_positive_volume() {
    assert_eq!(share_of_volume(Some(500), Some(1_000)), Some(50.0));
    assert_eq!(share_of_volume(Some(500), Some(0)), None);
    assert_eq!(share_of_volume(None, Some(1_000)), None);
    assert_eq!(lots(-1_500), -2);
    assert_eq!(lots(1_499), 1);
}
