use crate::{config::SETTINGS, internal::logging, internal::util::http};
use anyhow::*;
use serde::{Deserialize, Serialize};
use std::result::Result::Ok;

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

pub async fn send_to_allowed(msg: &str) -> Result<()> {
    for id in SETTINGS.bot.telegram.allowed.keys() {
        let payload = SendMessageRequest {
            chat_id: *id,
            text: msg,
        };

        send_message(payload).await?
    }

    Ok(())
}

pub async fn send_message<'a>(payload: SendMessageRequest<'_>) -> Result<()> {
    let api_url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        SETTINGS.bot.telegram.token
    );

    match http::request_post_use_json::<SendMessageRequest, SendMessageResponse>(
        &api_url,
        None,
        Some(&payload),
    )
    .await
    {
        Ok(_response) => {}
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to do_request_post_with_json because: {:?}",
                why
            ));
        }
    }

    Ok(())
}
