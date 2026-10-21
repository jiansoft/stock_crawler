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
    let recent = vec![BrokerPeriodNet {
        stock_symbol: "2330".to_owned(),
        main_net: -20_000,
        buy_days: 1,
        sell_days: 4,
    }];
    let body = response(Some(day), vec![chip(), empty], (5, recent));
    assert_eq!(body.broker_recent_days, 5);
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
        tsmc.broker_recent,
        Some(PortfolioChipBrokerRecent {
            main_net: -20_000,
            buy_days: 1,
            sell_days: 4,
        })
    );
    assert_eq!(
        tsmc.reasons,
        ["foreign_heavy", "foreign_streak", "trust_streak", "broker"]
    );

    let quiet = &body.items[1];
    assert!(quiet.reasons.is_empty() && quiet.broker.is_none() && quiet.broker_recent.is_none());
    assert!(quiet.foreign_share.is_none() && quiet.margin_change.is_none());
    assert_eq!(quiet.foreign_streak, 0);

    let none = response(None, Vec::new(), (0, Vec::new()));
    assert!(none.date.is_none() && none.items.is_empty() && none.notable == 0);
    assert_eq!(none.broker_recent_days, 0);
}

#[test]
fn share_of_volume_needs_positive_volume() {
    assert_eq!(share_of_volume(Some(500), Some(1_000)), Some(50.0));
    assert_eq!(share_of_volume(Some(500), Some(0)), None);
    assert_eq!(share_of_volume(None, Some(1_000)), None);
    assert_eq!(lots(-1_500), -2);
    assert_eq!(lots(1_499), 1);
}

/// 回傳固定主力交易日的假倉儲；期間合計檢查收到的起訖日，並對每個代號回一筆。
struct FakeBrokers(Vec<NaiveDate>);

#[async_trait::async_trait]
impl HoldingBrokerQuery for FakeBrokers {
    async fn fetch_broker_dates(&self, until: NaiveDate, limit: usize) -> Result<Vec<NaiveDate>> {
        assert_eq!((until, limit), (day(20), BROKER_RECENT_DAYS));
        Ok(self.0.clone())
    }

    async fn fetch_broker_period(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        symbols: &[String],
    ) -> Result<Vec<BrokerPeriodNet>> {
        assert_eq!((from, to), (day(14), day(20)));
        Ok(symbols
            .iter()
            .map(|symbol| BrokerPeriodNet {
                stock_symbol: symbol.clone(),
                main_net: 1,
                buy_days: 1,
                sell_days: 0,
            })
            .collect())
    }
}

/// 期間從最舊的主力交易日到查詢日、代號為當天持股；完全沒有主力資料時不查期間。
#[tokio::test]
async fn broker_recent_spans_recent_broker_days() {
    let brokers = FakeBrokers(vec![day(20), day(17), day(14)]);
    let (days, nets) = broker_recent(&brokers, day(20), &[chip()])
        .await
        .expect("假倉儲不會失敗");
    assert_eq!(days, 3);
    assert_eq!(nets.len(), 1);
    assert_eq!(nets[0].stock_symbol, "2330");
    let (days, nets) = broker_recent(&FakeBrokers(Vec::new()), day(20), &[chip()])
        .await
        .expect("假倉儲不會失敗");
    assert!(days == 0 && nets.is_empty());
}
