//! 驗證佔位在發送之前、重複事件略過，以及資料庫故障時只有系統警報可繼續送出。

use super::*;
use crate::domain::notify::DeliveryClaim;
use anyhow::{Result, anyhow};
use std::sync::Mutex;

/// 記錄動作順序的儲存替身。
struct Store {
    steps: Mutex<Vec<&'static str>>,
    duplicate: bool,
    unavailable: bool,
    finish_fails: bool,
}

impl Store {
    fn new(duplicate: bool, unavailable: bool) -> Self {
        Self {
            steps: Mutex::new(Vec::new()),
            duplicate,
            unavailable,
            finish_fails: false,
        }
    }
}

/// 讓 tracing 巨集的欄位實際求值（沒有 subscriber 時不會執行）。
fn capture_logs() -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .finish(),
    )
}

#[async_trait]
impl NotifyDeliveryRepository for Store {
    async fn reserve(&self, _: &NotifyRoute, _: &str, _: &str) -> Result<Option<DeliveryClaim>> {
        self.steps.lock().expect("測試鎖").push("reserve");
        if self.unavailable {
            return Err(anyhow!("db down"));
        }
        Ok((!self.duplicate).then_some(DeliveryClaim {
            id: 1,
            announce: true,
        }))
    }
    async fn finish(&self, _: DeliveryClaim, _: &NotifyRoute, _: DeliveryOutcome) -> Result<()> {
        self.steps.lock().expect("測試鎖").push("finish");
        if self.finish_fails {
            return Err(anyhow!("db down"));
        }
        Ok(())
    }
    async fn reconcile_pending(&self) -> Result<u64> {
        Ok(0)
    }
}

/// 發送前檢查佔位已存在，並記錄實際本文。
struct Sender<'a> {
    store: &'a Store,
    messages: Mutex<Vec<String>>,
}

#[async_trait]
impl DeliveryTransport for Sender<'_> {
    async fn send(&self, _: Topic, _: i64, message: &str) -> Result<(), SendFailure> {
        let mut steps = self.store.steps.lock().expect("測試鎖");
        assert_eq!(steps.last(), Some(&"reserve"));
        steps.push("send");
        self.messages
            .lock()
            .expect("測試鎖")
            .push(message.to_string());
        Ok(())
    }
}

/// 測試路由不含私人資料。
fn route() -> NotifyRoute {
    NotifyRoute {
        member_id: 1,
        household_id: 1,
        channel_id: 1,
        chat_id: 101,
        holdings_view: "own".into(),
        announce_new_topic: true,
    }
}

/// 佔位、發送、結果更新的順序不可交換，且提示附加在本文之後。
#[tokio::test]
async fn reserve_precedes_send_and_finish_and_duplicate_does_not_send() {
    for duplicate in [false, true] {
        let store = Store::new(duplicate, false);
        let sender = Sender {
            store: &store,
            messages: Mutex::new(Vec::new()),
        };
        assert_eq!(
            deliver_with(
                Topic::HoldingsPayable,
                &route(),
                "key",
                "本文",
                &store,
                &sender
            )
            .await,
            !duplicate
        );
        let expected: &[&str] = if duplicate {
            &["reserve"]
        } else {
            &["reserve", "send", "finish"]
        };
        assert_eq!(*store.steps.lock().expect("測試鎖"), expected);
        if !duplicate {
            assert_eq!(
                *sender.messages.lock().expect("測試鎖"),
                ["本文\n這是新增的通知項目，可在通知設定關閉"]
            );
        }
    }
}

/// 無法寫送達紀錄時，市場與持股停止，系統警報仍使用已解析的路由送出。
#[tokio::test]
async fn database_failure_only_allows_system_alerts_to_continue() {
    let _logs = capture_logs();
    for topic in [
        Topic::HoldingsPayable,
        Topic::MarketWeekly,
        Topic::SystemCritical,
    ] {
        let store = Store::new(false, true);
        let sender = Sender {
            store: &store,
            messages: Mutex::new(Vec::new()),
        };
        assert_eq!(
            deliver_with(topic, &route(), "", "警報", &store, &sender).await,
            topic == Topic::SystemCritical
        );
        assert_eq!(
            sender.messages.lock().expect("測試鎖").len(),
            usize::from(topic == Topic::SystemCritical)
        );
    }
}

/// 空白本文不佔位也不送出。
#[tokio::test]
async fn blank_message_is_not_reserved_or_sent() {
    let store = Store::new(false, false);
    let sender = Sender {
        store: &store,
        messages: Mutex::new(Vec::new()),
    };
    assert!(!deliver_with(Topic::MarketWeekly, &route(), "k", "  \n", &store, &sender).await);
    assert!(store.steps.lock().expect("測試鎖").is_empty());
}

/// 結果寫回失敗時保留 pending（之後轉 unknown），不重送；送出本身仍算成功。
#[tokio::test]
async fn finish_failure_keeps_pending_and_does_not_resend() {
    let _logs = capture_logs();
    let store = Store {
        finish_fails: true,
        ..Store::new(false, false)
    };
    let sender = Sender {
        store: &store,
        messages: Mutex::new(Vec::new()),
    };
    assert!(deliver_with(Topic::MarketWeekly, &route(), "k", "本文", &store, &sender).await);
    assert_eq!(
        *store.steps.lock().expect("測試鎖"),
        ["reserve", "send", "finish"]
    );
}

/// 沒有註冊 Telegram adapter 時，實際傳送端回報失敗而不是假裝送達。
#[tokio::test]
async fn checked_transport_without_adapter_reports_failure() {
    assert!(
        CheckedTransport
            .send(Topic::MarketWeekly, 101, "本文")
            .await
            .is_err()
    );
}

/// 正式入口先在資料庫佔位再送：管道不存在（或已停用）時佔位不成立，不送出也不留紀錄。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn deliver_reserves_in_database_before_sending() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 deliver_reserves_in_database_before_sending：無資料庫連接");
        return;
    }
    const MEMBER: i64 = 979_901;
    let cleanup = || async {
        sqlx::query("DELETE FROM member_notify_delivery WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await
            .expect("清除測試送達紀錄");
    };
    cleanup().await;
    let _logs = capture_logs();
    let fake = NotifyRoute {
        member_id: MEMBER,
        channel_id: MEMBER,
        ..route()
    };
    assert!(!deliver(Topic::MarketWeekly, &fake, "deliver-test", "本文").await);
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM member_notify_delivery WHERE member_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(MEMBER)
    .fetch_optional(pool)
    .await
    .expect("讀取送達紀錄");
    cleanup().await;
    assert_eq!(status, None, "管道不存在時不應佔位");
}
