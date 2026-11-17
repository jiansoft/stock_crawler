//! 在隔離 PostgreSQL 驗證跨執行佔位、重跑、failed 重試與 unknown 保留佔位。

use super::*;
use crate::app::notify_delivery::{DeliveryTransport, deliver_with};
use crate::core::alert::{SendFailure, Topic};
use crate::infra::database;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 計數實際送出的次數，不使用真實 bot。
struct Transport {
    sends: AtomicUsize,
    fail: bool,
}

#[async_trait]
impl DeliveryTransport for Transport {
    async fn send(&self, _: Topic, _: i64, _: &str) -> Result<(), SendFailure> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(SendFailure {
                error_code: Some(403),
                disabled_reason: Some("blocked_by_user"),
            })
        } else {
            Ok(())
        }
    }
}

/// 建立本測試專用的會員與管道，結束時只刪除自己建立的列。
async fn fixture(pool: &PgPool) -> NotifyRoute {
    let member_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "Member" ("Account", "Name", household_id) VALUES ('', '送達測試', 1) RETURNING "Id""#,
    ).fetch_one(pool).await.expect("建立會員");
    let channel_id: i64 = sqlx::query_scalar(
        "INSERT INTO member_notify_channel (member_id, kind, address) VALUES ($1, 'telegram', $2) RETURNING id",
    ).bind(member_id).bind(format!("__delivery_{member_id}"))
        .fetch_one(pool).await.expect("建立管道");
    NotifyRoute {
        member_id,
        household_id: 1,
        channel_id,
        chat_id: member_id,
        announce_new_topic: false,
        holdings_view: "own".into(),
    }
}

/// 兩個並行執行只送一次；重跑不重送，pending 逾時轉 unknown 後也不能再佔位。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn concurrent_delivery_and_unknown_are_at_most_once() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection().clone();
    let route = fixture(&pool).await;
    let store = PgNotifyDeliveryRepository { pool: pool.clone() };
    let transport = Transport {
        sends: AtomicUsize::new(0),
        fail: false,
    };
    let send = || {
        deliver_with(
            Topic::HoldingsPayable,
            &route,
            "same-event",
            "持股",
            &store,
            &transport,
        )
    };
    let (a, b) = tokio::join!(send(), send());
    assert_ne!(a, b);
    assert!(!send().await);
    assert_eq!(transport.sends.load(Ordering::SeqCst), 1);
    let claim = store
        .reserve(&route, Topic::HoldingsPayable.code(), "crashed-event")
        .await
        .expect("佔位")
        .expect("新事件");
    sqlx::query("UPDATE member_notify_delivery SET created_at = now() - interval '31 minutes' WHERE id = $1")
        .bind(claim.id).execute(&pool).await.expect("模擬當機");
    assert!(store.reconcile_pending().await.expect("對帳") >= 1);
    // 遲到的失敗結果不能把 unknown 改回 failed、釋放佔位。
    store
        .finish(
            claim,
            &route,
            DeliveryOutcome::Failed {
                error_code: Some(500),
                disabled_reason: None,
            },
        )
        .await
        .expect("遲到結果");
    assert!(
        store
            .reserve(&route, Topic::HoldingsPayable.code(), "crashed-event")
            .await
            .expect("重跑")
            .is_none()
    );
    cleanup(&pool, &route).await;
}

/// failed 可重試；403 停用後即使快取仍有該路由也不再送出。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn failed_is_retryable_and_blocked_channel_is_disabled() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection().clone();
    let route = fixture(&pool).await;
    let store = PgNotifyDeliveryRepository { pool: pool.clone() };
    let topic = Topic::HoldingsPayable;
    let claim = store
        .reserve(&route, topic.code(), "retry")
        .await
        .expect("佔位")
        .expect("新事件");
    store
        .finish(
            claim,
            &route,
            DeliveryOutcome::Failed {
                error_code: Some(500),
                disabled_reason: None,
            },
        )
        .await
        .expect("失敗");
    assert!(
        store
            .reserve(&route, topic.code(), "retry")
            .await
            .expect("重試")
            .is_some()
    );
    let transport = Transport {
        sends: AtomicUsize::new(0),
        fail: true,
    };
    assert!(!deliver_with(topic, &route, "blocked", "通知", &store, &transport).await);
    assert!(!deliver_with(topic, &route, "blocked-again", "通知", &store, &transport).await);
    assert_eq!(transport.sends.load(Ordering::SeqCst), 1);
    let state: (bool, String, i32) = sqlx::query_as(
        "SELECT enabled, disabled_reason, fail_count FROM member_notify_channel WHERE id = $1",
    )
    .bind(route.channel_id)
    .fetch_one(&pool)
    .await
    .expect("管道狀態");
    assert_eq!(state, (false, "blocked_by_user".into(), 2));
    cleanup(&pool, &route).await;
}

/// 初始路由不提示；背景補訂的首次成功送達才提示，後續事件不再提示。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn only_backfilled_topics_announce_the_first_successful_delivery() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection().clone();
    let mut route = fixture(&pool).await;
    let store = PgNotifyDeliveryRepository { pool: pool.clone() };
    let initial = store
        .reserve(&route, "test.initial", "initial")
        .await
        .expect("既有路由")
        .expect("新事件");
    assert!(!initial.announce);
    route.announce_new_topic = true;
    let first = store
        .reserve(&route, "test.new", "1")
        .await
        .expect("補訂")
        .expect("首次");
    assert!(first.announce);
    store
        .finish(first, &route, DeliveryOutcome::Sent)
        .await
        .expect("成功送達");
    assert!(
        !store
            .reserve(&route, "test.new", "2")
            .await
            .expect("第二次")
            .expect("新事件")
            .announce
    );
    // 個人價格追蹤的空 key 不由送達紀錄去重；兩次創新極端都必須能佔位。
    let personal_first = store
        .reserve(&route, Topic::PersonalPriceTrace.code(), "")
        .await
        .expect("個人類第一次")
        .expect("取得佔位");
    store
        .finish(personal_first, &route, DeliveryOutcome::Sent)
        .await
        .expect("個人類送達");
    let personal_second = store
        .reserve(&route, Topic::PersonalPriceTrace.code(), "")
        .await
        .expect("個人類下一次")
        .expect("不以空 key 去重");
    assert_ne!(personal_first.id, personal_second.id);
    cleanup(&pool, &route).await;
}

/// 清除本測試建立的送達紀錄與會員；管道由外來鍵一併刪除。
async fn cleanup(pool: &PgPool, route: &NotifyRoute) {
    sqlx::query("DELETE FROM member_notify_delivery WHERE member_id = $1")
        .bind(route.member_id)
        .execute(pool)
        .await
        .expect("清除送達紀錄");
    sqlx::query(r#"DELETE FROM "Member" WHERE "Id" = $1"#)
        .bind(route.member_id)
        .execute(pool)
        .await
        .expect("清除測試會員");
}
