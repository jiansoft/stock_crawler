//! Telegram 訊息防震 (Debouncer) 發送器的測試。

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
