use crate::{config::SETTINGS, internal::util::http, logging};
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

pub async fn send_message<'a>(payload : SendMessageRequest<'_>) -> Result<()> {
    let api_url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        SETTINGS.bot.telegram.token
    );

    match http::do_request_post_use_json::<SendMessageRequest, SendMessageResponse>(&api_url, &payload).await {
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
