use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use futures::future::join_all;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::{config::SETTINGS, util::http};

static TELEGRAM: Lazy<Arc<OnceLock<Telegram>>> = Lazy::new(|| Arc::new(OnceLock::new()));

struct Telegram {
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

    pub async fn send(&self, message: &str) -> Result<()> {
        let futures: Vec<_> = SETTINGS
            .bot
            .telegram
            .allowed
            .keys()
            .map(|id| self.send_message(SendMessageRequest::new(*id, message)))
            .collect();

        join_all(futures)
            .await
            .into_iter()
            .find(|res| res.is_err())
            .unwrap_or_else(|| Ok(()))
    }

    async fn send_message(&self, payload: SendMessageRequest<'_>) -> Result<()> {
        http::post_use_json::<SendMessageRequest, SendMessageResponse>(
            &self.send_message_url,
            None,
            Some(&payload),
        )
        .await
        .map_err(|err| anyhow!("Failed to send_message because: {:?}", err))?;

        Ok(())
    }
}

impl Default for Telegram {
    fn default() -> Self {
        Self::new()
    }
}

fn get_client() -> Result<&'static Telegram> {
    Ok(TELEGRAM.get_or_init(Telegram::new))
}

#[derive(Serialize, Deserialize)]
struct SendMessageResponse {
    ok: bool,
    result: Option<Message>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    message_id: i64,
}

#[derive(Serialize)]
pub struct SendMessageRequest<'a> {
    pub chat_id: i64,
    pub text: &'a str,
}

impl<'a> SendMessageRequest<'a> {
    pub fn new(chat_id: i64, text: &'a str) -> SendMessageRequest<'_> {
        SendMessageRequest { chat_id, text }
    }
}

pub async fn send(msg: &str) -> Result<()> {
    get_client()?.send(msg).await
}

#[cfg(test)]
mod tests {
    use std::env;

    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_send_message() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 test_send_message".to_string());
        let msg = format!(
            "test_send_message \r\nRust OS/Arch: {}/{}\r\n",
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
    }
}
