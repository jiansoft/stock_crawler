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

    notify(&idle, &recorder, &FakeFundamentals::default(), &[], day(16))
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
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn notify_settles_previous_period_on_rebalance_day() {
    dotenvy::dotenv().ok();
    let strategy = AlertStrategy {
        name: "甲".to_owned(),
        query: "from=2019-01-02".to_owned(),
    };
    let recorder = FakeRecorder::default();
    let settled = FakeGateway {
        settlement: Some(settlement(50)),
        ..gateway(Some(rebalance()))
    };
    notify(
        &settled,
        &recorder,
        &FakeFundamentals::default(),
        std::slice::from_ref(&strategy),
        day(16),
    )
    .await
    .expect("換股通知成功");
    assert_eq!(
        settled.settled.lock().expect("鎖未中毒").as_slice(),
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
        &[strategy],
        day(16),
    )
    .await
    .expect("結算失敗不影響通知");
    assert_eq!(failing.settled.lock().expect("鎖未中毒").len(), 1);
    assert_eq!(recorder.saved.lock().expect("鎖未中毒").len(), 2);
}
