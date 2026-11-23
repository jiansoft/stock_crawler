//! 系統告警/通知的抽象介面（port）。
//!
//! ## 為什麼需要這個模組
//!
//! 依 DDD 分層，依賴方向應該由外往內（interfaces → app → domain，core 為共用
//! 基礎）。但舊版的 `core::util::http` 與多個 `infra::crawler` 模組直接呼叫
//! `interfaces::bot::telegram`——形成「內層依賴外層」的反向耦合，
//! 導致核心模組難以單元測試、無法替換通知管道，也不能單獨抽成 library。
//!
//! ## 解法：port / adapter
//!
//! - 這裡（core）只定義「會發送告警」的抽象介面 [`AlertSink`]（port），
//!   以及一個全域註冊點。core/infra 只依賴這個抽象。
//! - 具體實作（送到 Telegram）由 `interfaces::bot::telegram` 提供（adapter），
//!   並在 `main` 啟動時透過 [`register_alert_sink`] 注入。
//! - 未註冊時（例如單元測試），告警內容降級為 warning log，不會 panic。
//!
//! ## 通知項目（會員系統計畫 7.3、7.6）
//!
//! 每一種推送都必須帶 [`Topic`]，依受眾分三個入口：[`notify_system`]（系統類，以警報格式送出）、
//! [`notify_market`]（市場類，所有訂閱者內容相同）、持股與個人類的 `app::notify::notify_holdings`
//! （依收件者範圍各自產生內容，經 [`send_to_chat`] 送到各收件者的聊天室）。

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

pub use crate::core::notify_topic::Topic;

/// 發送失敗的最小資訊，不攜帶 Telegram URL、Token 或通知內容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("通知發送失敗（代碼 {error_code:?}）")]
pub struct SendFailure {
    /// Telegram 錯誤碼；連線失敗時為空。
    pub error_code: Option<i32>,
    /// 管道已失效的原因；其餘錯誤只累計失敗次數。
    pub disabled_reason: Option<&'static str>,
}

/// 告警發送介面（port）。
///
/// 由外層（interfaces）提供實作並於啟動時註冊；core/infra 透過 [`notify_system`]、[`notify_system_text`]、
/// [`notify_market`] 間接使用，不需知道訊息實際送往何處。
#[async_trait]
pub trait AlertSink: Send + Sync {
    /// 以警報格式發送（標題＋詳情）。
    async fn send_alert(&self, topic: Topic, title: &str, message: &str);

    /// 發送已組好的訊息（設定檔內的聊天室）。
    async fn send_message(&self, topic: Topic, message: &str);

    /// 發送已組好的訊息到指定聊天室（依訂閱分送的收件者，7.6）。
    async fn send_to_chat(&self, topic: Topic, chat_id: i64, message: &str);

    /// 回報聊天室的實際送達結果；Telegram adapter 覆寫以保留錯誤碼。
    async fn send_to_chat_checked(
        &self,
        topic: Topic,
        chat_id: i64,
        message: &str,
    ) -> Result<(), SendFailure> {
        self.send_to_chat(topic, chat_id, message).await;
        Ok(())
    }

    /// 直接送設定檔的聊天室，避免再次解析路由而形成遞迴。
    async fn send_legacy(&self, topic: Topic, message: &str) {
        self.send_message(topic, message).await;
    }
}

/// 全域告警管道。`OnceLock` 保證只會被成功註冊一次。
static ALERT_SINK: OnceLock<Arc<dyn AlertSink>> = OnceLock::new();

/// 註冊全域告警管道，應於 `main` 啟動流程早期呼叫一次。
///
/// 重複註冊會被忽略並記 warning——第一個註冊者獲勝，
/// 避免執行中途悄悄替換通知管道造成困惑。
pub fn register_alert_sink(sink: Arc<dyn AlertSink>) {
    if ALERT_SINK.set(sink).is_err() {
        tracing::warn!("alert sink already registered; duplicate registration ignored");
    }
}

/// 系統類通知（取代舊的 `send_alert`）：以「標題＋詳情」的警報格式送出。
///
/// 格式由入口決定、分送由 [`Topic`] 決定；少數市場類項目沿用警報格式（例如 `market.strategy_edge`），
/// 也走這個入口。
///
/// 尚未註冊 sink 時（例如單元測試、啟動極早期），內容降級為 warning log，
/// 確保告警不會讓核心流程失敗，也不會無聲消失。
pub async fn notify_system(topic: Topic, title: &str, detail: &str) {
    match ALERT_SINK.get() {
        Some(sink) => sink.send_alert(topic, title, detail).await,
        None => {
            tracing::warn!(
                "alert sink not registered; alert dropped: [{}] {title}: {detail}",
                topic.code()
            );
        }
    }
}

/// 系統類通知的純文字版本：訊息已自行組好（例如啟動通知、資料健康週報、爬蟲來源異常），不套警報格式。
pub async fn notify_system_text(topic: Topic, message: &str) {
    send_message(topic, message).await;
}

/// 市場類通知：所有訂閱者內容相同，不含任何人的持股。
///
/// 尚未註冊 sink 時的行為同 [`notify_system`]：降級為 warning log。
pub async fn notify_market(topic: Topic, message: &str) {
    send_message(topic, message).await;
}

/// 依訂閱分送時送到某位收件者的聊天室（`app::notify` 使用）。
///
/// 尚未註冊 sink 時降級為 warning log（不記錄訊息內容：持股內容只屬於收件者）。
pub async fn send_to_chat(topic: Topic, chat_id: i64, message: &str) {
    match ALERT_SINK.get() {
        Some(sink) => sink.send_to_chat(topic, chat_id, message).await,
        None => {
            tracing::warn!(
                "alert sink not registered; routed message dropped: [{}] chat {chat_id}",
                topic.code()
            );
        }
    }
}

/// 送到設定檔內的聊天室（尚未依訂閱分送的主題，`app::notify` 使用）。
pub async fn send_legacy(topic: Topic, message: &str) {
    if let Some(sink) = ALERT_SINK.get() {
        sink.send_legacy(topic, message).await;
    } else {
        tracing::warn!(
            "alert sink not registered; legacy message dropped: [{}]",
            topic.code()
        );
    }
}

/// 發送並取得結果；沒有 adapter 時回失敗，不能記為已送達。
pub async fn send_to_chat_checked(
    topic: Topic,
    chat_id: i64,
    message: &str,
) -> Result<(), SendFailure> {
    match ALERT_SINK.get() {
        Some(sink) => sink.send_to_chat_checked(topic, chat_id, message).await,
        None => Err(SendFailure {
            error_code: None,
            disabled_reason: None,
        }),
    }
}

async fn send_message(topic: Topic, message: &str) {
    match ALERT_SINK.get() {
        Some(sink) => sink.send_message(topic, message).await,
        None => {
            tracing::warn!(
                "alert sink not registered; message dropped: [{}] {message}",
                topic.code()
            );
        }
    }
}

#[cfg(test)]
mod tests;
