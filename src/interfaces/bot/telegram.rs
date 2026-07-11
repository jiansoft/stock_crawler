use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use chrono::Local;
use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::{core::config::SETTINGS, core::util::http};

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
    pub async fn send(&self, message: &str) -> Result<SendMessageResponse> {
        let allowed_ids = SETTINGS.bot.telegram.allowed.keys();
        let futures =
            allowed_ids.map(|id| self.send_message(SendMessageRequest::new(*id, message)));
        let results = join_all(futures).await;

        // 尋找是否有成功發送且 API 返回 ok = true 的結果
        let first_ok = results
            .iter()
            .find_map(|r| r.as_ref().ok().filter(|res| res.ok));

        if let Some(resp) = first_ok {
            return Ok(resp.clone());
        }

        // 如果發送失敗（可能因為 MarkdownV2 解析錯誤，例如 status code 400 Bad Request），
        // 則執行降級重試機制：清除轉義用的反斜線，改用純文字模式發送。
        tracing::warn!(
            "{}",
            "Telegram message failed or returned error. Retrying with plain-text fallback..."
                .to_string(),
        );

        // 移除所有 Markdown 轉義字元，以便於以純文字模式清晰顯示
        let clean_msg = message.replace("\\", "");
        let fallback_futures = SETTINGS.bot.telegram.allowed.keys().map(|id| {
            let mut req = SendMessageRequest::new(*id, &clean_msg);
            req.parse_mode = ""; // 設定 parse_mode 為空，使其以純文字模式發送，不解析任何 markdown 標記
            self.send_message(req)
        });
        let fallback_results = join_all(fallback_futures).await;

        // 返回第一個成功的降級發送結果
        fallback_results
            .into_iter()
            .find_map(|result| result.ok())
            .ok_or_else(|| {
                anyhow!("Failed to send message to any recipient even after plain-text fallback")
            })
    }

    fn send_message<'a>(
        &'a self,
        payload: SendMessageRequest<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<SendMessageResponse>> + Send + 'a>>
    {
        Box::pin(async move {
            http::post_use_json::<SendMessageRequest, SendMessageResponse>(
                &self.send_message_url,
                None,
                Some(&payload),
            )
            .await
            .map_err(|err| anyhow!("Failed to send_message because: {:?}", err))
        })
    }

    /// 跳脫 Telegram `MarkdownV2` 保留字元。
    pub fn escape_markdown_v2(text: impl Into<String>) -> String {
        const SPECIALS: &[char] = &[
            '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.',
            '!',
        ];

        let text = text.into();
        let mut result = String::with_capacity(text.len() * 2); // 預留更多空間避免重新分配

        for ch in text.chars() {
            if SPECIALS.contains(&ch) {
                result.push('\\');
            }
            result.push(ch);
        }
        result
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
            parse_mode: "MarkdownV2",
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
    match client.send(msg).await {
        Ok(rep) => {
            if !rep.ok {
                let error_code = rep
                    .error_code
                    .as_ref()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let desc = rep.description.as_deref().unwrap_or("No description");
                tracing::error!(
                    "Telegram API responded with error code {error_code}: {desc}\n{msg}"
                );
            }
        }
        Err(error) => {
            tracing::error!("Failed to send a message to telegram because {error:}");
        }
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
        Telegram::escape_markdown_v2(alert_title),
        Telegram::escape_markdown_v2(Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        Telegram::escape_markdown_v2(details)
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
            Telegram::escape_markdown_v2(env::consts::OS),
            Telegram::escape_markdown_v2(env::consts::ARCH)
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

    /// 驗證 MarkdownV2 跳脫規則。
    #[test]
    fn test_escape_markdown_v2() {
        let input = "Hello_World*Test[link](url)";
        let expected = "Hello\\_World\\*Test\\[link\\]\\(url\\)";
        assert_eq!(Telegram::escape_markdown_v2(input), expected);
    }
}
