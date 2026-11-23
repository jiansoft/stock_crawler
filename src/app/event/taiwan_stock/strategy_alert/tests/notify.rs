//! 通知流程：換股紀錄、失敗後繼續、只在換股日發送與結算上一期。

use super::*;

/// 換股紀錄：新買進在前、續抱在後，賣出的不記；沒有名稱的策略以環境變數名稱記錄。
#[test]
fn rebalance_record_keeps_added_then_kept() {
    let strategy = AlertStrategy {
        name: "最佳組合".to_owned(),
        query: "from=2019-01-02".to_owned(),
    };
    let core = StrategyRebalance {
        etf_weight: 50,
        cash: 10.0,
        ..rebalance()
    };
    assert_eq!(
        rebalance_record(&strategy, &core),
        StrategyRebalanceRecord {
            preset_name: "最佳組合".to_owned(),
            date: day(16),
            query: "from=2019-01-02".to_owned(),
            hold_days: 20,
            etf_weight: 50,
            cash: 10.0,
            market_filtered: false,
            holdings: vec![
                RecordedHolding {
                    symbol: "2330".to_owned(),
                    name: Some("台積電".to_owned()),
                    weight: 50.0,
                },
                RecordedHolding {
                    symbol: "2317".to_owned(),
                    name: Some("鴻海".to_owned()),
                    weight: 50.0,
                },
            ],
        }
    );
    let unnamed = AlertStrategy {
        name: String::new(),
        ..strategy
    };
    assert_eq!(
        rebalance_record(&unnamed, &rebalance()).preset_name,
        UNNAMED_STRATEGY
    );
}

/// 逐一執行：今天不換股的不發、失敗的記下但不中斷其他策略，最後回傳錯誤；沒有策略不需要 gateway。
#[tokio::test]
async fn notify_continues_after_failures() {
    let strategy = |name: &str| AlertStrategy {
        name: name.to_owned(),
        query: format!("from=2019-01-02&name={name}"),
    };
    let recorder = FakeRecorder::default();
    let idle = gateway(None);
    notify(
        &idle,
        &recorder,
        &FakeFundamentals::default(),
        &FakeOverlap::default(),
        &[strategy("甲"), strategy("乙")],
        day(16),
    )
    .await
    .expect("都不換股");
    assert_eq!(idle.queries.lock().expect("鎖未中毒").len(), 2);

    let failing = FakeGateway {
        fail: true,
        ..gateway(None)
    };
    let error = notify(
        &failing,
        &recorder,
        &FakeFundamentals::default(),
        &FakeOverlap::default(),
        &[strategy("甲"), strategy("乙")],
        day(16),
    )
    .await
    .expect_err("回測失敗");
    let message = format!("{error:#}");
    assert!(
        message.contains("甲：") && message.contains("乙："),
        "{message}"
    );
    assert_eq!(failing.queries.lock().expect("鎖未中毒").len(), 2);

    notify(
        &idle,
        &recorder,
        &FakeFundamentals::default(),
        &FakeOverlap::default(),
        &[],
        day(16),
    )
    .await
    .expect("沒有策略");
    // 都不是換股日（或失敗），不寫換股紀錄。
    assert!(recorder.saved.lock().expect("鎖未中毒").is_empty());
}

/// 今天是換股日才發；最後一期已滿或換股日不是今天都不發；回測錯誤往上傳。
#[tokio::test]
async fn run_only_alerts_on_rebalance_day() {
    let fake = gateway(Some(rebalance()));
    let message = rebalance_today(&fake, "from=2019-01-02", day(16))
        .await
        .expect("回測成功");
    assert!(message.is_some_and(|rebalance| rebalance.added[0].symbol == "2330"));
    assert_eq!(
        fake.queries.lock().expect("鎖未中毒").as_slice(),
        ["from=2019-01-02"]
    );

    let message = rebalance_today(&fake, "from=2019-01-02", day(17))
        .await
        .expect("回測成功");
    assert!(message.is_none());

    let none = gateway(None);
    assert!(
        rebalance_today(&none, "from=2019-01-02", day(16))
            .await
            .expect("回測成功")
            .is_none()
    );

    let failing = FakeGateway {
        fail: true,
        ..gateway(None)
    };
    assert!(rebalance_today(&failing, "from=x", day(16)).await.is_err());
}

/// 換股日：寫入紀錄、依策略名稱結算上一期後發通知；結算失敗只略過結算，不影響通知與其他策略。
/// 持股對照只經持股段（替身）產生，市場段不讀實際持股，因此不需要資料庫（7.3 防漏）。
#[tokio::test]
async fn notify_settles_previous_period_on_rebalance_day() {
    let strategy = AlertStrategy {
        name: "甲".to_owned(),
        query: "from=2019-01-02".to_owned(),
    };
    let recorder = FakeRecorder::default();
    let overlap = FakeOverlap::default();
    let settled = FakeGateway {
        settlement: Some(settlement(50)),
        ..gateway(Some(rebalance()))
    };
    notify(
        &settled,
        &recorder,
        &FakeFundamentals::default(),
        &overlap,
        std::slice::from_ref(&strategy),
        day(16),
    )
    .await
    .expect("換股通知成功");
    assert_eq!(
        settled.settled.lock().expect("鎖未中毒").as_slice(),
        [("甲".to_owned(), day(16))]
    );
    // 每次換股恰好交給持股段一次（帶策略名稱）；持股查詢只在持股段的 build 內。
    assert_eq!(
        overlap.calls.lock().expect("鎖未中毒").as_slice(),
        [("甲".to_owned(), day(16))]
    );

    let failing = FakeGateway {
        settlement_fail: true,
        ..gateway(Some(rebalance()))
    };
    notify(
        &failing,
        &recorder,
        &FakeFundamentals::failing(),
        &FakeOverlap::default(),
        &[strategy],
        day(16),
    )
    .await
    .expect("結算失敗不影響通知");
    assert_eq!(failing.settled.lock().expect("鎖未中毒").len(), 1);
    assert_eq!(recorder.saved.lock().expect("鎖未中毒").len(), 2);
}

/// 不是換股日時不產生持股段。
#[tokio::test]
async fn overlap_only_on_rebalance_day() {
    let overlap = FakeOverlap::default();
    notify(
        &gateway(None),
        &FakeRecorder::default(),
        &FakeFundamentals::default(),
        &overlap,
        &[AlertStrategy {
            name: "甲".to_owned(),
            query: "from=2019-01-02".to_owned(),
        }],
        day(16),
    )
    .await
    .expect("不換股");
    assert!(overlap.calls.lock().expect("鎖未中毒").is_empty());
}
