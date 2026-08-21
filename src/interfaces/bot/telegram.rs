use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, anyhow};
use chrono::Local;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

// MarkdownV2 跳脫工具（text::escape_markdown_v2）住在 core 層：
// app 層組訊息與這裡的 adapter 都需要同一套跳脫規則，
// 放在 core 才不會讓 app 反向依賴 interfaces（詳見該函式的 rustdoc）。
use crate::{
    core::config::SETTINGS,
    core::util::{http, text},
};

//static TELEGRAM: Lazy<Arc<OnceLock<Telegram>>> = Lazy::new(|| Arc::new(OnceLock::new()));
static TELEGRAM: OnceLock<Telegram> = OnceLock::new();

/// Telegram Bot API 客戶端。
pub struct Telegram {
    /// `sendMessage` API 的完整 URL。
    send_message_url: String,
}

/// `sendMessage` API 回應內容。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendMessageResponse {
    /// Telegram API 是否成功處理請求。
    pub ok: bool,
    /// 成功時回傳的訊息內容。
    pub result: Option<Message>,
    /// 失敗時的錯誤代碼。
    pub error_code: Option<i32>,
    /// 失敗時的錯誤描述。
    pub description: Option<String>,
    /// 失敗時的補充參數，目前只用到頻率限制的 `retry_after`。
    #[serde(default)]
    pub parameters: Option<ResponseParameters>,
}

/// Telegram 在部分錯誤回應中附帶的補充參數。
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ResponseParameters {
    /// 觸發頻率限制（429）時，還需要等待幾秒才能重送。
    #[serde(default)]
    pub retry_after: Option<u64>,
}

/// Telegram 訊息物件的最小欄位表示。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    /// Telegram 訊息 ID。
    message_id: i64,
}

/// 發送 Telegram 訊息時使用的請求內容。
#[derive(Serialize)]
pub struct SendMessageRequest<'a> {
    /// 目標聊天室 ID。
    pub chat_id: i64,
    /// 訊息內容。
    pub text: &'a str,
    /// Telegram 解析模式。
    #[serde(rename = "parse_mode")]
    pub parse_mode: &'a str,
}

/// Telegram 的 MarkdownV2 解析模式。
const MARKDOWN_V2: &str = "MarkdownV2";

/// 純文字模式：`parse_mode` 留空時 Telegram 不解析任何標記。
const PLAIN_TEXT: &str = "";

/// 兩則訊息之間的最小間隔。
///
/// Telegram 對同一個 chat 的限制約為 1 msg/s，對整個 bot token 約為 30 msg/s。
/// 由於所有出站訊息都會經過 [`acquire_send_slot`] 排成一列，這個間隔同時滿足
/// 兩條限制。多留 100ms 安全邊際，因為官方門檻是浮動且未文件化的。
const MIN_SEND_INTERVAL: Duration = Duration::from_millis(1_100);

/// 上一則訊息實際送出的時間點；`None` 代表本次程序啟動後尚未送過。
///
/// 用 `tokio::sync::Mutex` 而非 `std::sync::Mutex`：持有期間必須跨越
/// `sleep().await`，同步鎖會擋住整個 executor 執行緒。
///
/// 時間戳用 `tokio::time::Instant` 而非 `std::time::Instant`，
/// 這樣測試才能以 `start_paused` 的虛擬時鐘驗證節流，不必真的等上一秒。
static LAST_SENT_AT: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

/// 取得一個發送許可：等到距離上一則訊息已滿 [`MIN_SEND_INTERVAL`] 為止。
///
/// 鎖會一路持有到等待結束，因此並行的呼叫端會被排成一列依序取得許可，
/// 不會出現多個 task 同時醒來一起打 API 的情況。
///
/// 這是原本缺少的一環：先前每個呼叫點各自 `join_all` 併發打 Telegram API，
/// 一份被切成多段的長報表會在一秒內把整批送出，必然撞上 429。
async fn acquire_send_slot() {
    let mut last_sent_at = LAST_SENT_AT.lock().await;

    if let Some(previous) = *last_sent_at {
        let elapsed = previous.elapsed();
        if elapsed < MIN_SEND_INTERVAL {
            tokio::time::sleep(MIN_SEND_INTERVAL - elapsed).await;
        }
    }

    *last_sent_at = Some(Instant::now());
}

impl Telegram {
    /// 建立 Telegram API 客戶端。
    pub fn new() -> Self {
        Self {
            send_message_url: format!(
                "https://api.telegram.org/bot{}/sendMessage",
                SETTINGS.bot.telegram.token
            ),
        }
    }

    /// 將同一則訊息送給設定檔中的所有允許接收者。
    ///
    /// 失敗時依序嘗試兩種補救：
    ///
    /// 1. 撞上 429 頻率限制——依 Telegram 回報的 `retry_after` 等滿後原樣重送一次。
    ///    此時不做純文字降級：訊息本身沒問題，降級只會再撞一次限制。
    /// 2. 其他失敗（多半是 MarkdownV2 解析錯誤的 400）——移除跳脫用的反斜線，
    ///    改以純文字重送，讓使用者至少看得到內容。
    ///
    /// 兩種補救各自最多重試一次，避免在通知管道上做無界重試。
    pub async fn send(&self, message: &str) -> Result<SendMessageResponse> {
        match self.broadcast(message, MARKDOWN_V2).await {
            Ok(resp) => return Ok(resp),
            Err(Some(retry_after)) => {
                // 多等一秒：retry_after 是「還要等幾秒」的下限而非精確值。
                let wait = Duration::from_secs(retry_after + 1);
                tracing::warn!("Telegram 觸發頻率限制，{wait:?} 後重送");
                tokio::time::sleep(wait).await;

                if let Ok(resp) = self.broadcast(message, MARKDOWN_V2).await {
                    return Ok(resp);
                }
            }
            Err(None) => {}
        }

        tracing::warn!(
            "Telegram message failed or returned error. Retrying with plain-text fallback..."
        );

        // 移除所有 Markdown 跳脫字元，以便於以純文字模式清晰顯示。
        let clean_msg = message.replace('\\', "");
        self.broadcast(&clean_msg, PLAIN_TEXT).await.map_err(|_| {
            anyhow!("Failed to send message to any recipient even after plain-text fallback")
        })
    }

    /// 以指定的 `parse_mode` 把訊息送給所有允許的接收者，回傳第一則成功的回應。
    ///
    /// 一律送完全部接收者才回傳，不會因為前一位成功就略過其餘的人。
    ///
    /// 送出是序列的而非 `join_all` 併發：每則都要先取得 [`acquire_send_slot`]
    /// 的許可，併發也只是全部塞在鎖上排隊，徒增同時打 API 的機會。
    ///
    /// # 回傳
    ///
    /// * `Ok(resp)` —— 至少一位接收者送達。
    /// * `Err(Some(secs))` —— 全部失敗，且其中有 429；`secs` 為觀察到最長的等待秒數。
    /// * `Err(None)` —— 全部失敗且與頻率限制無關。
    async fn broadcast(
        &self,
        message: &str,
        parse_mode: &str,
    ) -> std::result::Result<SendMessageResponse, Option<u64>> {
        let mut first_ok: Option<SendMessageResponse> = None;
        let mut retry_after: Option<u64> = None;

        for id in SETTINGS.bot.telegram.allowed.keys() {
            let mut req = SendMessageRequest::new(*id, message);
            req.parse_mode = parse_mode;

            match self.send_message(req).await {
                Ok(resp) if resp.ok => {
                    if first_ok.is_none() {
                        first_ok = Some(resp);
                    }
                }
                Ok(resp) => {
                    if resp.error_code == Some(429) {
                        // 沒帶 retry_after 時保守地等一秒再說。
                        let secs = resp
                            .parameters
                            .as_ref()
                            .and_then(|p| p.retry_after)
                            .unwrap_or(1);
                        retry_after = Some(retry_after.map_or(secs, |current| current.max(secs)));
                    }

                    let error_code = resp
                        .error_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let desc = resp.description.as_deref().unwrap_or("No description");
                    tracing::error!("Telegram API 回應錯誤 chat_id={id} code={error_code}: {desc}");
                }
                Err(err) => {
                    tracing::error!("Telegram 發送失敗 chat_id={id}: {err:?}");
                }
            }
        }

        first_ok.ok_or(retry_after)
    }

    fn send_message<'a>(
        &'a self,
        payload: SendMessageRequest<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<SendMessageResponse>> + Send + 'a>>
    {
        Box::pin(async move {
            // 節流放在最底層：不論來自 broadcast 的哪一輪（首次、429 重送、
            // 純文字降級），每一次實際的 API 呼叫都會被排進同一條佇列。
            acquire_send_slot().await;

            http::post_use_json::<SendMessageRequest, SendMessageResponse>(
                &self.send_message_url,
                None,
                Some(&payload),
            )
            .await
            .map_err(|err| anyhow!("Failed to send_message because: {:?}", err))
        })
    }
}

impl Default for Telegram {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SendMessageRequest<'a> {
    /// 建立 `sendMessage` 請求，預設採用 `MarkdownV2`。
    pub fn new(chat_id: i64, text: &'a str) -> SendMessageRequest<'a> {
        SendMessageRequest {
            chat_id,
            text,
            parse_mode: MARKDOWN_V2,
        }
    }
}

/// 取得全域共用的 Telegram client。
fn get_client() -> &'static Telegram {
    TELEGRAM.get_or_init(Telegram::new)
}

/// Telegram `sendMessage` 的文字長度上限（entities 解析後的字元數）。
///
/// 官方限制為 4096；這裡預留一點餘裕，避免邊界情況下的計數誤差。
const MAX_MESSAGE_LEN: usize = 4000;

/// 依行為單位切割訊息，確保每個分段都不超過 Telegram 的長度限制。
///
/// 以整行為切割單位，避免把 MarkdownV2 的連結或跳脫序列從中間截斷。
/// 若單行本身就超過上限（極端情況），才退回逐字切割，避免無窮迴圈或直接送不出去。
fn split_message_into_chunks(msg: &str, limit: usize) -> Vec<String> {
    if msg.chars().count() <= limit {
        return vec![msg.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in msg.split_inclusive('\n') {
        if line.chars().count() > limit {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let mut slice = String::new();
            for ch in line.chars() {
                if slice.chars().count() >= limit {
                    chunks.push(std::mem::take(&mut slice));
                }
                slice.push(ch);
            }
            if !slice.is_empty() {
                chunks.push(slice);
            }
            continue;
        }

        if !current.is_empty() && current.chars().count() + line.chars().count() > limit {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// 異步發送 Telegram 消息
///
/// 若訊息長度超過 Telegram 的 4096 字元上限，會依行切割成多則訊息依序發送，
/// 避免整則訊息因為過長而被 Bot API 以 400 Bad Request 拒絕。
///
/// # Arguments
///
/// * `msg` - 要發送的消息內容
pub async fn send(msg: &str) {
    if msg.trim().is_empty() {
        return;
    }

    for chunk in split_message_into_chunks(msg, MAX_MESSAGE_LEN) {
        send_single(&chunk).await;
    }
}

/// 發送單一則（已確保長度合法的）Telegram 消息。
async fn send_single(msg: &str) {
    let client = get_client();
    // 個別接收者的失敗已由 Telegram::broadcast 記錄；這裡只在所有補救
    // 手段（429 重送、純文字降級）都用盡後，補一則帶原文的錯誤 log。
    if let Err(error) = client.send(msg).await {
        tracing::error!("Failed to send a message to telegram because {error:}\n{msg}");
    }
}

/// 發送關鍵警報訊息至 Telegram。
///
/// 此函數主要用於背景任務、資料庫異常或關鍵流程失敗時，向 Telegram 發送顯眼的警報。
///
/// # 參數
/// * `alert_title` - 警報的標題
/// * `details` - 警報的詳細內容或錯誤堆疊
pub async fn send_alert(alert_title: &str, details: &str) {
    let msg = format!(
        "⚠️ *【系統關鍵警報】*\n*標題*︰{}\n*時間*︰{}\n*詳情*︰\n```\n{}\n```",
        text::escape_markdown_v2(alert_title),
        text::escape_markdown_v2(Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        text::escape_markdown_v2(details)
    );
    send(&msg).await;
}

/// 把 Telegram 通知接上 `core::alert::AlertSink` port 的 adapter。
///
/// 依 DDD 分層，core/infra 不應該直接呼叫 interfaces（內層依賴外層的
/// 反向耦合）。因此 core 只定義抽象的 [`crate::core::alert::AlertSink`]，
/// 由這個 adapter 提供 Telegram 實作，並在 `main` 啟動時註冊——
/// core/infra 呼叫 `core::alert::send_alert` 時，訊息才會實際送到 Telegram。
pub struct TelegramAlertSink;

#[async_trait::async_trait]
impl crate::core::alert::AlertSink for TelegramAlertSink {
    async fn send_alert(&self, title: &str, message: &str) {
        send_alert(title, message).await;
    }

    async fn send_message(&self, message: &str) {
        send(message).await;
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::time::Duration;

    use tokio::time;

    use crate::infra::cache::SHARE;

    use super::*;

    /// 驗證 Telegram API 實際送信流程。
    #[tokio::test]
    #[ignore]
    async fn test_send_message() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 test_send_message");
        let msg = format!(
            "test_send_message Rust OSArch: {}{}",
            text::escape_markdown_v2(env::consts::OS),
            text::escape_markdown_v2(env::consts::ARCH)
        );
        get_client().send(&msg).await.expect("TODO: panic message");
        // let _ = send_to_allowed(&msg).await;

        tracing::debug!("結束 test_send_message");
        time::sleep(Duration::from_secs(1)).await;
    }

    /// 驗證訊息未超過上限時不會被切割。
    #[test]
    fn test_split_message_into_chunks_keeps_short_message_intact() {
        let msg = "line1\nline2\nline3\n";
        let chunks = split_message_into_chunks(msg, 4000);
        assert_eq!(chunks, vec![msg.to_string()]);
    }

    /// 驗證超過上限的訊息會依整行切割成多段，且每段都不超過上限。
    #[test]
    fn test_split_message_into_chunks_splits_by_line_when_too_long() {
        let line = "x".repeat(30);
        let msg = std::iter::repeat_n(line.clone(), 10)
            .collect::<Vec<_>>()
            .join("\n");
        let limit = 100;

        let chunks = split_message_into_chunks(&msg, limit);

        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= limit);
        }
        // 每一行內容都完整保留在切割後的某個分段中，沒有被腰斬。
        assert_eq!(
            chunks.iter().flat_map(|c| c.lines()).count(),
            msg.lines().count()
        );
    }

    /// 驗證單一行本身就超過上限的極端情況，會退回逐字切割而不是卡死或整段丟棄。
    #[test]
    fn test_split_message_into_chunks_falls_back_to_char_split_for_oversized_line() {
        let msg = "a".repeat(250);
        let limit = 100;

        let chunks = split_message_into_chunks(&msg, limit);

        assert_eq!(chunks.len(), 3);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= limit);
        }
        assert_eq!(chunks.iter().map(|c| c.chars().count()).sum::<usize>(), 250);
    }

    // MarkdownV2 跳脫規則的測試已隨函式本體移至 `core::util::text::tests`。

    /// 驗證節拍器：第一則不等待，之後每則都要等滿 `MIN_SEND_INTERVAL`。
    ///
    /// 以 `start_paused` 的虛擬時鐘執行，因此不會真的花掉數秒。
    /// `LAST_SENT_AT` 是程序層級的全域狀態，只有這個測試會動它。
    #[tokio::test(start_paused = true)]
    async fn test_acquire_send_slot_enforces_min_interval() {
        let start = Instant::now();

        // 尚未送過任何訊息，第一則不該被延遲。
        acquire_send_slot().await;
        assert_eq!(start.elapsed(), Duration::ZERO);

        // 緊接著的第二、三則各自要等滿一個間隔。
        acquire_send_slot().await;
        assert_eq!(start.elapsed(), MIN_SEND_INTERVAL);

        acquire_send_slot().await;
        assert_eq!(start.elapsed(), MIN_SEND_INTERVAL * 2);

        // 距離上次已超過間隔時不該再等。
        time::sleep(MIN_SEND_INTERVAL * 2).await;
        let before = Instant::now();
        acquire_send_slot().await;
        assert_eq!(before.elapsed(), Duration::ZERO);
    }

    /// 驗證 429 回應的 `parameters.retry_after` 能被正確解析。
    ///
    /// 這個欄位先前完全沒有定義，撞上頻率限制時無從得知該等多久。
    #[test]
    fn test_deserialize_rate_limited_response() {
        let body = r#"{
            "ok": false,
            "error_code": 429,
            "description": "Too Many Requests: retry after 17",
            "parameters": { "retry_after": 17 }
        }"#;

        let resp: SendMessageResponse = serde_json::from_str(body).expect("回應應可解析");

        assert!(!resp.ok);
        assert_eq!(resp.error_code, Some(429));
        assert_eq!(
            resp.parameters.and_then(|p| p.retry_after),
            Some(17),
            "retry_after 必須被解析出來，否則無從得知該等多久"
        );
    }

    /// 驗證成功回應（沒有 parameters 欄位）仍可解析。
    ///
    /// `parameters` 是選填欄位，缺少時不得讓整個回應解析失敗。
    #[test]
    fn test_deserialize_success_response_without_parameters() {
        let body = r#"{ "ok": true, "result": { "message_id": 1234 } }"#;

        let resp: SendMessageResponse = serde_json::from_str(body).expect("回應應可解析");

        assert!(resp.ok);
        assert!(resp.parameters.is_none());
        assert!(resp.result.is_some());
    }
}
