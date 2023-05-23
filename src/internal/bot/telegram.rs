use crate::internal::{config::SETTINGS, logging, util::http};
use anyhow::*;
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};
use std::{result::Result::Ok, sync::Arc};

static TELEGRAM: Lazy<Arc<OnceCell<Telegram>>> = Lazy::new(|| Arc::new(OnceCell::new()));

struct Telegram {
    send_message_url: String,
}

impl Telegram {
    pub fn new() -> Self {
        Telegram {
            send_message_url: format!(
                "https://api.telegram.org/bot{}/sendMessage",
                SETTINGS.bot.telegram.token
            ),
        }
    }

    pub async fn send(&self, msg: &str) -> Result<()> {
        for id in SETTINGS.bot.telegram.allowed.keys() {
            let payload = SendMessageRequest {
                chat_id: *id,
                text: msg,
            };

            self.send_message(payload).await?
        }

        Ok(())
    }

    async fn send_message<'a>(&self, payload: SendMessageRequest<'_>) -> Result<()> {
        if let Err(why) = http::request_post_use_json::<SendMessageRequest, SendMessageResponse>(
            &self.send_message_url,
            None,
            Some(&payload),
        )
        .await
        {
            logging::error_file_async(format!(
                "Failed to http::request_post_use_json because: {:?}",
                why
            ));
        }

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

pub async fn send(msg: &str) -> Result<()> {
    get_client()?.send(msg).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::SHARE;
    use std::env;

    #[tokio::test]
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
