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

/// 異步發送 Telegram 消息
///
/// # Arguments
///
/// * `msg` - 要發送的消息內容
pub async fn send(msg: &str) {
    if msg.trim().is_empty() {
        return;
    }
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
        dotenv::dotenv().ok();
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

    /// 驗證 MarkdownV2 跳脫規則。
    #[test]
    fn test_escape_markdown_v2() {
        let input = "Hello_World*Test[link](url)";
        let expected = "Hello\\_World\\*Test\\[link\\]\\(url\\)";
        assert_eq!(Telegram::escape_markdown_v2(input), expected);
    }
}
