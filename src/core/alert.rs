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

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

/// 告警發送介面（port）。
///
/// 由外層（interfaces）提供實作並於啟動時註冊；core/infra 透過
/// [`send_alert`] / [`send_message`] 間接使用，不需知道訊息實際送往何處。
#[async_trait]
pub trait AlertSink: Send + Sync {
    /// 發送「系統關鍵警報」等級的訊息（例如 IP 被封鎖、服務異常）。
    async fn send_alert(&self, title: &str, message: &str);

    /// 發送一般通知訊息。
    async fn send_message(&self, message: &str);
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

/// 發送系統關鍵警報。
///
/// 尚未註冊 sink 時（例如單元測試、啟動極早期），內容降級為 warning log，
/// 確保告警不會讓核心流程失敗，也不會無聲消失。
pub async fn send_alert(title: &str, message: &str) {
    match ALERT_SINK.get() {
        Some(sink) => sink.send_alert(title, message).await,
        None => {
            tracing::warn!("alert sink not registered; alert dropped: {title}: {message}");
        }
    }
}

/// 發送一般通知訊息。
///
/// 尚未註冊 sink 時的行為同 [`send_alert`]：降級為 warning log。
pub async fn send_message(message: &str) {
    match ALERT_SINK.get() {
        Some(sink) => sink.send_message(message).await,
        None => {
            tracing::warn!("alert sink not registered; message dropped: {message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 把訊息記錄到記憶體的測試用 sink。
    struct RecordingSink {
        received: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl AlertSink for RecordingSink {
        async fn send_alert(&self, title: &str, message: &str) {
            self.received
                .lock()
                .unwrap()
                .push(format!("alert:{title}:{message}"));
        }

        async fn send_message(&self, message: &str) {
            self.received.lock().unwrap().push(format!("msg:{message}"));
        }
    }

    /// 驗證註冊後訊息會送達 sink，且重複註冊不會 panic（第一個註冊者獲勝）。
    ///
    /// 注意：ALERT_SINK 是全域單例且無法重設，因此註冊相關驗證集中在
    /// 同一個測試內，避免測試間互相影響。
    #[tokio::test]
    async fn registered_sink_receives_messages_and_duplicate_registration_is_ignored() {
        let sink = Arc::new(RecordingSink {
            received: Mutex::new(Vec::new()),
        });
        register_alert_sink(sink.clone());

        send_alert("標題", "內容").await;
        send_message("一般訊息").await;

        // 重複註冊：不 panic、原 sink 仍生效。
        register_alert_sink(Arc::new(RecordingSink {
            received: Mutex::new(Vec::new()),
        }));
        send_message("第二則").await;

        let received = sink.received.lock().unwrap();
        assert_eq!(
            *received,
            vec![
                "alert:標題:內容".to_string(),
                "msg:一般訊息".to_string(),
                "msg:第二則".to_string(),
            ]
        );
    }
}
