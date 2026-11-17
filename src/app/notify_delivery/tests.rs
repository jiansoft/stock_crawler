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
        let store = Store {
            steps: Mutex::new(Vec::new()),
            duplicate,
            unavailable: false,
        };
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
    for topic in [
        Topic::HoldingsPayable,
        Topic::MarketWeekly,
        Topic::SystemCritical,
    ] {
        let store = Store {
            steps: Mutex::new(Vec::new()),
            duplicate: false,
            unavailable: true,
        };
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
