//! Telegram 訊息防震 (Debouncer) 發送器。
//!
//! 將短時間內密集產生的 Telegram 訊息合併後批次發送，節省 API 呼叫次數，
//! 並維持與重構前批次發送通知相同的視覺呈現。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;

/// <summary>
/// Telegram 訊息的防震 (Debouncer) 發送器。
/// 用於將短時間內密集產生的 Telegram 訊息合併後批次發送，以節省 API 呼叫次數，
/// 並維持與重構前批次發送通知相同的視覺呈現。
/// </summary>
pub(super) struct TelegramDebouncer {
    buffer: Arc<Mutex<Vec<String>>>,
    epoch: Arc<AtomicU64>,
}

impl TelegramDebouncer {
    /// 建立新的 Debouncer 實例。
    pub(super) fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 將單一訊息加入發送緩衝區，並重置定時器。
    /// 在 500 毫秒內若無新訊息加入，則會觸發批次發送。
    pub(super) async fn add_message(&self, msg: String) {
        {
            let mut buf = self.buffer.lock().await;
            buf.push(msg);
        }
        let current_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;

        let buffer = self.buffer.clone();
        let epoch = self.epoch.clone();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if epoch.load(Ordering::SeqCst) == current_epoch {
                let mut buf = buffer.lock().await;
                if !buf.is_empty() {
                    let merged = buf.join("\r\n");
                    buf.clear();
                    let _ = crate::interfaces::bot::telegram::send(&merged).await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_telegram_debouncer() {
        let debouncer = TelegramDebouncer::new();
        debouncer.add_message("msg1".to_string()).await;
        debouncer.add_message("msg2".to_string()).await;

        // 此時 buffer 應該有 2 個 msg
        {
            let buf = debouncer.buffer.lock().await;
            assert_eq!(buf.len(), 2);
        }

        // 等待超過 500 毫秒，批次發送應該會執行且排空 buffer
        tokio::time::sleep(tokio::time::Duration::from_millis(700)).await;
        {
            let buf = debouncer.buffer.lock().await;
            assert!(buf.is_empty());
        }
    }
}
