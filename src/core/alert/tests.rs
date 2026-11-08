//! 系統告警/通知的抽象介面（port）的測試。

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
