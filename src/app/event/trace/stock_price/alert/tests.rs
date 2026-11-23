//! 追蹤條件的邊界判斷、警報去重（記憶體 TTL 與 Redis 雙層）與警報訊息的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 驗證價格區間判斷邏輯可正確處理雙邊界、單邊界與未設定情況。
#[test]
fn test_is_within_boundary() {
    // 設定高低標 (500 ~ 600)
    let mut trace = PriceTrace {
        member_id: 1,
        stock_symbol: "2330".to_string(),
        floor: dec!(500),
        ceiling: dec!(600),
    };

    // 邊界測試
    assert!(is_within_boundary(&trace, dec!(550)));
    assert!(is_within_boundary(&trace, dec!(500)));
    assert!(is_within_boundary(&trace, dec!(600)));
    assert!(!is_within_boundary(&trace, dec!(499.9)));
    assert!(!is_within_boundary(&trace, dec!(600.1)));

    // 僅設定低標 (>= 500)
    trace.ceiling = Decimal::ZERO;
    assert!(is_within_boundary(&trace, dec!(500)));
    assert!(is_within_boundary(&trace, dec!(1000)));
    assert!(!is_within_boundary(&trace, dec!(499.9)));

    // 僅設定高標 (<= 600)
    trace.floor = Decimal::ZERO;
    trace.ceiling = dec!(600);
    assert!(is_within_boundary(&trace, dec!(600)));
    assert!(is_within_boundary(&trace, dec!(0.1)));
    assert!(!is_within_boundary(&trace, dec!(600.1)));

    // 皆未設定
    trace.ceiling = Decimal::ZERO;
    assert!(is_within_boundary(&trace, dec!(123)));
}

/// 價格在區間內時直接回傳不需通知，不碰任何去重快取。
#[tokio::test]
async fn alert_on_price_boundary_ignores_prices_within_range() {
    let target = PriceTrace {
        member_id: 1,
        stock_symbol: "79984".to_string(),
        floor: dec!(10),
        ceiling: dec!(20),
    };
    let sent = alert_on_price_boundary(target, dec!(15), EvaluationSource::PriceEvent, None)
        .await
        .expect("判斷邊界");
    assert!(!sent);
}

/// 跌破低標第一次通知；同價或較高的價格在時間窗內不再通知，創新低才再通知。
/// 測試用假代號，結束時刪除 Redis 去重紀錄。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn alert_on_price_boundary_notifies_only_on_new_lows() {
    dotenvy::dotenv().ok();
    let redis = &crate::infra::nosql::redis::CLIENT;
    if redis.ping().await.is_err() {
        println!("跳過 alert_on_price_boundary_notifies_only_on_new_lows：無 Redis 連線");
        return;
    }
    let pool = crate::infra::database::get_connection();
    let member_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "Member" ("Account", "Name", household_id)
           VALUES ('__price_alert_lows__', '價格測試', 1) RETURNING "Id""#,
    )
    .fetch_one(pool)
    .await
    .expect("建立測試會員");
    let target = PriceTrace {
        member_id,
        stock_symbol: "79984".to_string(),
        floor: dec!(10),
        ceiling: Decimal::ZERO,
    };
    let key = build_trace_notification_key(&target, "floor");
    let _ = redis.delete(&key).await;

    let alert = |price| {
        alert_on_price_boundary(
            target.clone(),
            price,
            EvaluationSource::Reconciliation,
            Some("Test"),
        )
    };
    assert!(alert(dec!(9)).await.expect("第一次跌破"));
    assert!(!alert(dec!(9)).await.expect("同價"));
    assert!(!alert(dec!(9.5)).await.expect("未創新低"));
    assert!(alert(dec!(8.5)).await.expect("創新低"));

    let member_cleanup = sqlx::query(r#"DELETE FROM "Member" WHERE "Id" = $1"#)
        .bind(member_id)
        .execute(pool)
        .await;
    redis.delete(&key).await.expect("清除去重紀錄");
    member_cleanup.expect("清除測試會員");
}

/// 只用正式記憶體極端值比較的替身，讓分送測試不必連線 Redis。
struct MemoryPriceDedup;

#[async_trait::async_trait]
impl PriceDedup for MemoryPriceDedup {
    async fn should_notify(&self, key: &str, price: Decimal, lower: bool) -> bool {
        TTL.trace_quote_notify_if_more_extreme(
            key.to_string(),
            price,
            lower,
            Duration::from_secs(3600),
        )
    }
}

/// 記錄實際分送結果的替身，禁止測試呼叫 Telegram。
#[derive(Default)]
struct TraceSender(std::sync::Mutex<Vec<(i64, String)>>);

#[async_trait::async_trait]
impl crate::app::notify::HoldingsSender for TraceSender {
    async fn send_legacy(&self, _: alert::Topic, _: &str) {
        panic!("已切換的測試不應走舊聊天室");
    }
    async fn send_to_chat(&self, _: alert::Topic, chat_id: i64, message: &str) {
        self.0
            .lock()
            .expect("測試鎖")
            .push((chat_id, message.into()));
    }
}

/// 提供固定的測試通知路由。
struct TraceRoutes(Vec<crate::domain::notify::NotifyRoute>);

#[async_trait::async_trait]
impl crate::domain::notify::NotifyRouteRepository for TraceRoutes {
    async fn fetch_routes(&self, _: &str) -> Result<Vec<crate::domain::notify::NotifyRoute>> {
        Ok(self.0.clone())
    }
}

/// 經完整共用分送流程：先只觸發甲，之後兩人都觸發；甲的舊極端值不壓掉乙首次通知。
#[tokio::test]
async fn personal_trace_dispatches_only_to_triggered_subscribers() {
    let targets = [
        PriceTrace::new(79731, "79731".into(), dec!(100), dec!(200)),
        PriceTrace::new(79732, "79731".into(), dec!(80), dec!(200)),
    ];
    let owners = [(79731, "甲".into()), (79732, "乙".into())];
    let sender = TraceSender::default();
    let routes = TraceRoutes(
        [79731, 79732]
            .into_iter()
            .map(|id| trace_route(1, id, "own"))
            .collect(),
    );
    for (price, expected) in [(dec!(90), 1), (dec!(79), 2), (dec!(79), 0)] {
        let event = PriceAlert {
            targets: &targets,
            current_price: price,
            source_site: None,
        };
        let summary = crate::app::notify::dispatch_for_recipient(
            alert::Topic::PersonalPriceTrace,
            "同一事件識別不控制個人通知去重",
            |view, recipient| event.build(view, recipient, &owners, &MemoryPriceDedup),
            Some(&routes),
            &sender,
            true,
        )
        .await;
        assert_eq!(summary.sent, expected);
    }
    let messages = sender.0.lock().expect("測試鎖");
    assert_eq!(
        messages.iter().map(|(chat, _)| *chat).collect::<Vec<_>>(),
        [79831, 79831, 79832]
    );
    assert!(messages[0].1.contains("低於最低價:100"));
    assert!(messages[2].1.contains("低於最低價:80"));
}

fn trace_route(
    household_id: i64,
    member_id: i64,
    holdings_view: &str,
) -> crate::domain::notify::NotifyRoute {
    crate::domain::notify::NotifyRoute {
        member_id,
        household_id,
        channel_id: member_id,
        announce_new_topic: false,
        holdings_view: holdings_view.into(),
        chat_id: member_id + 100,
    }
}

/// 同一報價觸及兩個家庭的條件；以 PostgreSQL 解析擁有者並驗證實際訊息內容。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn trace_notifications_keep_two_households_separate() {
    use crate::infra::database;
    use crate::infra::database::repository::member;

    let mut tx = database::get_tx().await.expect("開啟隔離測試交易");
    let household_b: i64 = sqlx::query_scalar(
        "INSERT INTO member_household (name) VALUES ('__trace_712_b__') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("建立第二家庭");
    let mut routes = Vec::new();
    let mut targets = Vec::new();
    for (household, account, floor) in [
        (1, "通知甲員", dec!(101)),
        (household_b, "通知乙員", dec!(87)),
    ] {
        let member_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "Member" ("Account", "Name", household_id) VALUES ($1,$1,$2) RETURNING "Id""#,
        )
        .bind(account)
        .bind(household)
        .fetch_one(&mut *tx)
        .await
        .expect("建立家庭會員");
        routes.push(trace_route(household, member_id, "household"));
        targets.push(PriceTrace::new(
            member_id,
            "IAM712TRACE".into(),
            floor,
            dec!(300),
        ));
    }
    let routes = TraceRoutes(routes);
    let sender = TraceSender::default();
    let event = PriceAlert {
        targets: &targets,
        current_price: dec!(50),
        source_site: None,
    };
    let tx = tokio::sync::Mutex::new(tx);
    let summary = crate::app::notify::dispatch_for_recipient(
        alert::Topic::PersonalPriceTrace,
        "兩家收到同一筆報價",
        |view, recipient| {
            let (event, tx) = (&event, &tx);
            async move {
                let owners = {
                    let mut tx = tx.lock().await;
                    member::fetch_names_from(&mut **tx, &view)
                        .await?
                        .into_iter()
                        .collect::<Vec<_>>()
                };
                event
                    .build(view, recipient, &owners, &MemoryPriceDedup)
                    .await
            }
        },
        Some(&routes),
        &sender,
        true,
    )
    .await;
    assert_eq!(summary.sent, 2);
    for (route, own, other) in [
        (&routes.0[0], "通知甲員", "通知乙員"),
        (&routes.0[1], "通知乙員", "通知甲員"),
    ] {
        let messages = sender.0.lock().expect("測試鎖");
        let (_, message) = messages
            .iter()
            .find(|(chat_id, _)| *chat_id == route.chat_id)
            .expect("送到該家庭的聊天室");
        assert!(message.contains(own), "應標明本人家庭的觸發條件：{message}");
        assert!(
            !message.contains(other),
            "應排除另一家庭的觸發條件：{message}"
        );
    }
    tx.into_inner().rollback().await.expect("回滾測試資料");
}

/// 報價只跌破甲的條件，乙本人範圍與其他家庭都沒有內容；家庭通知註明並跳脫條件擁有者。
#[tokio::test]
async fn personal_trace_filters_triggered_conditions_and_labels_household_owner() {
    let targets = [
        PriceTrace::new(79701, "79701".into(), dec!(100), dec!(200)),
        PriceTrace::new(79702, "79701".into(), dec!(80), dec!(200)),
    ];
    let owners = [(79701, "甲_[家人]".into()), (79702, "乙".into())];
    let event = PriceAlert {
        targets: &targets,
        current_price: dec!(90),
        source_site: Some("Test"),
    };
    let own = event
        .build(
            HoldingView::member(1, 79701),
            Some(79701),
            &owners,
            &MemoryPriceDedup,
        )
        .await
        .expect("甲的通知")
        .expect("甲觸發");
    assert!(own.contains("低於最低價:100"));
    assert!(!own.contains("甲"));
    assert!(!own.contains("乙"));
    assert!(
        event
            .build(
                HoldingView::member(1, 79702),
                Some(79702),
                &owners,
                &MemoryPriceDedup
            )
            .await
            .expect("乙的通知")
            .is_none()
    );
    assert!(
        event
            .build(
                HoldingView::household(2),
                Some(79703),
                &[],
                &MemoryPriceDedup
            )
            .await
            .expect("其他家庭")
            .is_none()
    );
    let family = event
        .build(
            HoldingView::household(1),
            Some(79704),
            &owners,
            &MemoryPriceDedup,
        )
        .await
        .expect("家庭通知")
        .expect("含甲的條件");
    assert!(family.starts_with("甲\\_\\[家人\\] 的追蹤條件："));
    assert!(!family.contains("乙"));
}

/// 相同家庭範圍的兩位收件會員，連續價格同價、反彈、創新低時各自保有通知狀態。
#[tokio::test]
async fn personal_trace_extreme_state_is_independent_for_each_recipient() {
    let targets = [PriceTrace::new(79711, "79711".into(), dec!(100), dec!(200))];
    let owners = [(79711, "甲".into())];
    for (price, expected) in [
        (dec!(90), true),
        (dec!(90), false),
        (dec!(95), false),
        (dec!(89), true),
    ] {
        let event = PriceAlert {
            targets: &targets,
            current_price: price,
            source_site: None,
        };
        for recipient in [79711, 79712] {
            assert_eq!(
                event
                    .build(
                        HoldingView::household(1),
                        Some(recipient),
                        &owners,
                        &MemoryPriceDedup
                    )
                    .await
                    .expect("產生通知")
                    .is_some(),
                expected,
                "收件者 {recipient}，報價 {price}"
            );
        }
    }
}

/// 高標創新高才再發；未切換時訊息仍與原格式逐字相同，舊快取不壓掉新路由。
#[tokio::test]
async fn personal_trace_legacy_message_and_new_highs_keep_existing_behaviour() {
    let targets = [PriceTrace::new(79721, "79721".into(), dec!(100), dec!(200))];
    let owners = [(79721, "甲".into())];
    let event = PriceAlert {
        targets: &targets,
        current_price: dec!(210),
        source_site: Some("Test"),
    };
    let legacy = event
        .build(
            HoldingView::default_household(),
            None,
            &owners,
            &MemoryPriceDedup,
        )
        .await
        .expect("舊通道")
        .expect("首次觸發");
    assert_eq!(
        legacy,
        format_alert_message(&targets[0], dec!(210), Some("Test")).await
    );
    for (price, expected) in [
        (dec!(210), true),
        (dec!(210), false),
        (dec!(205), false),
        (dec!(211), true),
    ] {
        let event = PriceAlert {
            targets: &targets,
            current_price: price,
            source_site: Some("Test"),
        };
        assert_eq!(
            event
                .build(
                    HoldingView::member(1, 79721),
                    Some(79721),
                    &owners,
                    &MemoryPriceDedup
                )
                .await
                .expect("新路由")
                .is_some(),
            expected
        );
    }
}

#[test]
fn test_build_trace_notification_key_includes_boundary() {
    let trace = PriceTrace {
        member_id: 1,
        stock_symbol: "2330".to_string(),
        floor: dec!(25),
        ceiling: dec!(30),
    };

    // Key 僅含股票與邊界方向，不含價格（價格改以快取的值保存為極端值基準）。
    assert_eq!(
        build_trace_notification_key(&trace, "floor"),
        "Trace:1-2330-25-30:floor"
    );
    assert_eq!(
        build_trace_notification_key(&trace, "ceiling"),
        "Trace:1-2330-25-30:ceiling"
    );
}

/// 甲、乙以相同上下限追蹤同一檔時，去重鍵不同：甲的通知不會擋掉乙的通知。
#[test]
fn test_build_trace_notification_key_distinguishes_member() {
    let first = PriceTrace::new(1, "2330".to_string(), dec!(25), dec!(30));
    let second = PriceTrace::new(2, "2330".to_string(), dec!(25), dec!(30));
    assert_ne!(
        build_trace_notification_key(&first, "floor"),
        build_trace_notification_key(&second, "floor")
    );
}

#[test]
fn test_build_trace_notification_key_distinguishes_boundary() {
    let trace = PriceTrace {
        member_id: 1,
        stock_symbol: "2330".to_string(),
        floor: dec!(25),
        ceiling: dec!(30),
    };

    // 不同邊界方向應產生不同 key；同一方向不論價格皆為同一 key。
    assert_ne!(
        build_trace_notification_key(&trace, "floor"),
        build_trace_notification_key(&trace, "ceiling")
    );
}

#[tokio::test]
async fn test_format_alert_message_includes_source_site() {
    let trace = PriceTrace {
        member_id: 1,
        stock_symbol: "0050".to_string(),
        floor: dec!(0),
        ceiling: dec!(200),
    };

    let msg = format_alert_message(&trace, dec!(650), Some("Fugle")).await;

    assert!(msg.contains("超過最高價"));
    assert!(msg.contains("目前報價:650"));
    assert!(msg.contains("採集站點:Fugle"));
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_format_alert_message() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let trace = PriceTrace {
        member_id: 1,
        stock_symbol: "2330".to_string(),
        floor: dec!(500),
        ceiling: dec!(600),
    };

    // 觸發高標
    let msg = format_alert_message(&trace, dec!(650), Some("HiStock")).await;
    assert!(msg.contains("超過最高價"));
    assert!(msg.contains("目前報價:650"));
    assert!(msg.contains("採集站點:HiStock"));

    // 觸發低標
    let msg = msg_low(&trace, dec!(450), Some("Yahoo")).await;
    assert!(msg.contains("低於最低價"));
    assert!(msg.contains("目前報價:450"));
    assert!(msg.contains("採集站點:Yahoo"));
}

async fn msg_low(target: &PriceTrace, price: Decimal, source_site: Option<&str>) -> String {
    format_alert_message(target, price, source_site).await
}

#[tokio::test]
#[ignore]
async fn test_handle_price() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let trace = PriceTrace {
        member_id: 1,
        stock_symbol: "1303".to_string(),
        floor: dec!(70),
        ceiling: dec!(60),
    };

    let result = alert_on_price_boundary(
        trace,
        dec!(560),
        EvaluationSource::PriceEvent,
        Some("TestSite"),
    )
    .await;
    assert!(result.is_ok());
}
