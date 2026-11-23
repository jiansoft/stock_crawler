//! 可靠送達流程：佔位成功才送出，實際結果再寫回資料庫；系統警報在資料庫故障時仍能送達。

use async_trait::async_trait;

use crate::core::alert::{self, SendFailure, Topic};
use crate::core::notify_topic::Audience;
use crate::domain::notify::{DeliveryOutcome, NotifyDeliveryRepository, NotifyRoute};
use crate::infra::database;
use crate::infra::database::repository::notify_delivery::PgNotifyDeliveryRepository;

/// 真正送到聊天室的介面，測試以替身記錄，不呼叫 Telegram。
#[async_trait]
pub trait DeliveryTransport: Send + Sync {
    /// 發送完整訊息（含切段），回傳實際結果。
    async fn send(&self, topic: Topic, chat_id: i64, message: &str) -> Result<(), SendFailure>;
}

/// 經 core port 呼叫註冊的 Telegram adapter。
pub struct CheckedTransport;

#[async_trait]
impl DeliveryTransport for CheckedTransport {
    async fn send(&self, topic: Topic, chat_id: i64, message: &str) -> Result<(), SendFailure> {
        alert::send_to_chat_checked(topic, chat_id, message).await
    }
}

/// 正式環境的送達入口；個人價格追蹤的 key 已由呼叫端清空，沿用極端價格快取去重。
pub async fn deliver(topic: Topic, route: &NotifyRoute, key: &str, message: &str) -> bool {
    deliver_with(
        topic,
        route,
        key,
        message,
        &PgNotifyDeliveryRepository {
            pool: database::get_connection().clone(),
        },
        &CheckedTransport,
    )
    .await
}

/// 可注入儲存與傳送端的本體，確保所有發送都在佔位之後。
pub async fn deliver_with(
    topic: Topic,
    route: &NotifyRoute,
    key: &str,
    message: &str,
    store: &dyn NotifyDeliveryRepository,
    transport: &dyn DeliveryTransport,
) -> bool {
    if message.trim().is_empty() {
        return false;
    }
    let claim = match store.reserve(route, topic.code(), key).await {
        Ok(Some(claim)) => Some(claim),
        Ok(None) => return false,
        Err(error) => {
            tracing::error!(
                topic = topic.code(),
                channel_id = route.channel_id,
                "通知佔位失敗：{error:#}"
            );
            // 系統警報的路由可來自記憶體快取；不能因無法寫送達紀錄而使資料庫故障警報也失效。
            if topic.audience() != Audience::System {
                return false;
            }
            None
        }
    };
    let message = if claim.is_some_and(|claim| claim.announce) {
        format!("{message}\n這是新增的通知項目，可在通知設定關閉")
    } else {
        message.to_string()
    };
    let result = transport.send(topic, route.chat_id, &message).await;
    if let Some(claim) = claim {
        let outcome = match result {
            Ok(()) => DeliveryOutcome::Sent,
            Err(error) => DeliveryOutcome::Failed {
                error_code: error.error_code,
                disabled_reason: error.disabled_reason,
            },
        };
        if let Err(error) = store.finish(claim, route, outcome).await {
            // 保留 pending，之後轉為 unknown；絕不能因更新失敗再送一次。
            tracing::error!(delivery_id = claim.id, "通知結果更新失敗：{error:#}");
        }
    }
    result.is_ok()
}

/// 背景對帳每五分鐘處理逾時 pending，啟動時也立即執行一次。
pub fn spawn_reconciliation() {
    tokio::spawn(async {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            timer.tick().await;
            let store = PgNotifyDeliveryRepository {
                pool: database::get_connection().clone(),
            };
            match store.reconcile_pending().await {
                Ok(count) if count > 0 => tracing::info!(count, "通知逾時佔位已轉為 unknown"),
                Ok(_) => {}
                Err(error) => tracing::warn!("通知送達對帳失敗：{error:#}"),
            }
        }
    });
}

#[cfg(test)]
mod tests;
