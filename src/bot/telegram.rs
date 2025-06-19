use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use futures::future::join_all;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::{config::SETTINGS, logging, util::http};

static TELEGRAM: Lazy<Arc<OnceLock<Telegram>>> = Lazy::new(|| Arc::new(OnceLock::new()));

pub struct Telegram {
    send_message_url: String,
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
        //let escape_text = Telegram::escape_markdown_v2( message);

        let futures: Vec<_> = SETTINGS
            .bot
            .telegram
            .allowed
            .keys()
            .map(|id| self.send_message(SendMessageRequest::new(*id, message)))
            .collect();
        /* join_all(futures)
        .await
        .into_iter()
        .find(|res| res.is_err())
        .unwrap_or_else(|res| Ok(()))*/
        let results = join_all(futures).await;

        for result in results {
            match result {
                Ok(response) => return Ok(response),
                Err(_) => continue,
            }
        }

        Err(anyhow!("Failed to send message to any recipient"))
    }

    async fn send_message(&self, payload: SendMessageRequest<'_>) -> Result<SendMessageResponse> {
        let res = http::post_use_json::<SendMessageRequest, SendMessageResponse>(
            &self.send_message_url,
            None,
            Some(&payload),
        )
        .await
        .map_err(|err| anyhow!("Failed to send_message because: {:?}", err))?;
        Ok(res)
    }

    pub fn escape_markdown_v2(text: &str) -> String {
        let specials = r"_*[]()~`>#+-=|{}.!";
        let mut result = String::with_capacity(text.len());
        for ch in text.chars() {
            if specials.contains(ch) {
                result.push('\\');
            }
            result.push(ch);
        }
        result
    }

    /* fn escape_text(&self,parse_mode: &str, text: &str) -> String {
        let replacements: HashMap<&str, &str> = match parse_mode {
            "ModeHTML" => vec![("<", "&lt;"), (">", "&gt;"), ("&", "&amp;")].into_iter().collect(),
            "ModeMarkdown" => vec![("_", "\\_"), ("*", "\\*"), ("`", "\\`"), ("[", "\\[")].into_iter().collect(),
            "ModeMarkdownV2" => vec![
                ("_", "\\_"), ("*", "\\*"), ("[", "\\["), ("]", "\\]"), ("(", "\\("), (")", "\\)"),
                ("~", "\\~"), ("`", "\\`"), (">", "\\>"), ("#", "\\#"), ("+", "\\+"), ("-", "\\-"),
                ("=", "\\="), ("|", "\\|"), ("{", "\\{"), ("}", "\\}"), (".", "\\."), ("!", "\\!")
            ].into_iter().collect(),
            _ => return String::new(),
        };

        let mut escaped_text = text.to_string();
        for (from, to) in replacements {
            escaped_text = escaped_text.replace(from, to);
        }

        escaped_text
    }*/
}

impl Default for Telegram {
    fn default() -> Self {
        Self::new()
    }
}

fn get_client() -> Result<&'static Telegram> {
    Ok(TELEGRAM.get_or_init(Telegram::new))
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

impl<'a> SendMessageRequest<'a> {
    pub fn new(chat_id: i64, text: &'a str) -> SendMessageRequest<'a> {
        SendMessageRequest {
            chat_id,
            text,
            parse_mode: "MarkdownV2",
        }
    }
}

/// Asynchronously sends a message using a Telegram client.
///
/// This function attempts to retrieve a Telegram client and use it to send the provided message.
/// If either the retrieval or sending fails, an error is logged.
///
/// # Arguments
///
/// * `msg` - A string slice that holds the message to be sent.
///
/// # Errors
///
/// This function logs errors if it fails to get the Telegram client or send the message.
pub async fn send(msg: &str) {
    // Try to get a Telegram client
    match get_client() {
        Ok(client) => {
            // Try to send the message using the client
            if let Err(error) = client.send(msg).await {
                // Log an error if sending the message fails
                logging::error_file_async(format!(
                    "Failed to send message to telegram because {:?}",
                    error
                ));
            }
        }
        Err(error) => {
            // Log an error if getting the client fails
            logging::error_file_async(format!("Failed to get telegram client because {:?}", error));
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
            env::consts::OS,
            env::consts::ARCH
        );
        get_client()
            .expect("REASON")
            .send(&msg)
            .await
            .expect("TODO: panic message");
        // let _ = send_to_allowed(&msg).await;

        logging::debug_file_async("結束 test_send_message".to_string());
        time::sleep(Duration::from_secs(1)).await;
    }
}
