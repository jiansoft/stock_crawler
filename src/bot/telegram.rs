use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::{config::SETTINGS, logging, util::http};

//static TELEGRAM: Lazy<Arc<OnceLock<Telegram>>> = Lazy::new(|| Arc::new(OnceLock::new()));
static TELEGRAM: OnceLock<Telegram> = OnceLock::new();

pub struct Telegram {
    send_message_url: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendMessageResponse {
    pub ok: bool,
    pub result: Option<Message>,
    pub error_code: Option<i32>,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    message_id: i64,
}

#[derive(Serialize)]
pub struct SendMessageRequest<'a> {
    pub chat_id: i64,
    pub text: &'a str,
    #[serde(rename = "parse_mode")]
    pub parse_mode: &'a str,
}

impl Telegram {
    pub fn new() -> Self {
        Self {
            send_message_url: format!(
                "https://api.telegram.org/bot{}/sendMessage",
                SETTINGS.bot.telegram.token
            ),
        }
    }

    pub async fn send(&self, message: &str) -> Result<SendMessageResponse> {
        let allowed_ids = SETTINGS.bot.telegram.allowed.keys();
        let futures =
            allowed_ids.map(|id| self.send_message(SendMessageRequest::new(*id, message)));
        let results = join_all(futures).await;

        // 返回第一個成功的結果
        results
            .into_iter()
            .find_map(|result| result.ok())
            .ok_or_else(|| anyhow!("Failed to send message to any recipient"))
    }

    async fn send_message(&self, payload: SendMessageRequest<'_>) -> Result<SendMessageResponse> {
        http::post_use_json::<SendMessageRequest, SendMessageResponse>(
            &self.send_message_url,
            None,
            Some(&payload),
        )
        .await
        .map_err(|err| anyhow!("Failed to send_message because: {:?}", err))
    }

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
    pub fn new(chat_id: i64, text: &'a str) -> SendMessageRequest<'a> {
        SendMessageRequest {
            chat_id,
            text,
            parse_mode: "MarkdownV2",
        }
    }
}

fn get_client() -> &'static Telegram {
    TELEGRAM.get_or_init(Telegram::new)
}

/// 異步發送 Telegram 消息
///
/// # Arguments
///
/// * `msg` - 要發送的消息內容
pub async fn send(msg: &str) {
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
                logging::error_file_async(format!(
                    "Telegram API responded with error code {error_code}: {desc}\n{msg}"
                ));
            }
        }
        Err(error) => {
            logging::error_file_async(format!(
                "Failed to send a message to telegram because {error:}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::time::Duration;

    use tokio::time;

    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_send_message() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 test_send_message".to_string());
        let msg = format!(
            "test_send_message Rust OSArch: {}{}",
            Telegram::escape_markdown_v2(env::consts::OS),
            Telegram::escape_markdown_v2(env::consts::ARCH)
        );
        get_client().send(&msg).await.expect("TODO: panic message");
        // let _ = send_to_allowed(&msg).await;

        logging::debug_file_async("結束 test_send_message".to_string());
        time::sleep(Duration::from_secs(1)).await;
    }

    #[test]
    fn test_escape_markdown_v2() {
        let input = "Hello_World*Test[link](url)";
        let expected = "Hello\\_World\\*Test\\[link\\]\\(url\\)";
        assert_eq!(Telegram::escape_markdown_v2(input), expected);
    }
}
