use crate::core::util::datetime::TaipeiDateTime;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

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
    /// 允許接收通知的 chat id（設定檔 `bot.telegram.allowed`）。
    recipients: Vec<i64>,
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

/// 各聊天室與全域共用的行程內發送佇列。
mod send_queue;
use send_queue::SendQueue;
static SEND_QUEUE: std::sync::LazyLock<SendQueue> = std::sync::LazyLock::new(SendQueue::default);

impl Telegram {
    /// 建立 Telegram API 客戶端。
    pub fn new() -> Self {
        Self {
            send_message_url: format!(
                "https://api.telegram.org/bot{}/sendMessage",
                SETTINGS.bot.telegram.token
            ),
            recipients: SETTINGS.bot.telegram.allowed.keys().copied().collect(),
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
        self.send_to(&self.recipients, message).await
    }

    /// 與 [`Self::send`] 相同的補救流程，但送給指定的聊天室（依訂閱分送的收件者，7.6）。
    pub async fn send_to(&self, recipients: &[i64], message: &str) -> Result<SendMessageResponse> {
        match self.broadcast(recipients, message, MARKDOWN_V2).await {
            Ok(resp) => return Ok(resp),
            Err(Some(retry_after)) => {
                // 多等一秒：retry_after 是「還要等幾秒」的下限而非精確值。
                let wait = Duration::from_secs(retry_after + 1);
                tracing::warn!("Telegram 觸發頻率限制，{wait:?} 後重送");
                tokio::time::sleep(wait).await;

                if let Ok(resp) = self.broadcast(recipients, message, MARKDOWN_V2).await {
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
        self.broadcast(recipients, &clean_msg, PLAIN_TEXT)
            .await
            .map_err(|_| {
                anyhow!("Failed to send message to any recipient even after plain-text fallback")
            })
    }

    /// 以指定的 `parse_mode` 把訊息送給所有允許的接收者，回傳第一則成功的回應。
    ///
    /// 一律送完全部接收者才回傳，不會因為前一位成功就略過其餘的人。
    ///
    /// 送出是序列的而非 `join_all` 併發：每則都要先取得行程內佇列
    /// 的許可，併發也只是全部塞在鎖上排隊，徒增同時打 API 的機會。
    ///
    /// # 回傳
    ///
    /// * `Ok(resp)` —— 至少一位接收者送達。
    /// * `Err(Some(secs))` —— 全部失敗，且其中有 429；`secs` 為觀察到最長的等待秒數。
    /// * `Err(None)` —— 全部失敗且與頻率限制無關。
    async fn broadcast(
        &self,
        recipients: &[i64],
        message: &str,
        parse_mode: &str,
    ) -> std::result::Result<SendMessageResponse, Option<u64>> {
        let mut first_ok: Option<SendMessageResponse> = None;
        let mut retry_after: Option<u64> = None;

        for &id in recipients {
            let mut req = SendMessageRequest::new(id, message);
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

    /// 單一聊天室的可追蹤發送，保留 API 錯誤碼，失效管道不再嘗試純文字重送。
    async fn send_checked(
        &self,
        chat_id: i64,
        message: &str,
    ) -> Result<(), crate::core::alert::SendFailure> {
        let mut response = self
            .send_message(SendMessageRequest::new(chat_id, message))
            .await
            .ok();
        if response.as_ref().is_some_and(|r| r.ok) {
            return Ok(());
        }
        if response.as_ref().is_some_and(|r| r.error_code == Some(429)) {
            let seconds = response
                .as_ref()
                .and_then(|r| r.parameters.as_ref())
                .and_then(|p| p.retry_after)
                .unwrap_or(1)
                .saturating_add(1);
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            response = self
                .send_message(SendMessageRequest::new(chat_id, message))
                .await
                .ok();
            if response.as_ref().is_some_and(|r| r.ok) {
                return Ok(());
            }
        }
        let failure = send_failure(response.as_ref());
        if failure.disabled_reason.is_some() {
            return Err(failure);
        }
        let clean = message.replace('\\', "");
        let mut request = SendMessageRequest::new(chat_id, &clean);
        request.parse_mode = PLAIN_TEXT;
        let response = self.send_message(request).await.ok();
        if response.as_ref().is_some_and(|r| r.ok) {
            Ok(())
        } else {
            Err(send_failure(response.as_ref()))
        }
    }

    /// 所有初次發送、重送與純文字降級共用同一條出站佇列。
    fn send_message<'a>(
        &'a self,
        payload: SendMessageRequest<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<SendMessageResponse>> + Send + 'a>>
    {
        Box::pin(async move {
            // 節流放在最底層：不論來自 broadcast 的哪一輪（首次、429 重送、
            // 純文字降級），每一次實際的 API 呼叫都會被排進同一條佇列。
            SEND_QUEUE.acquire(payload.chat_id).await;

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

/// 只擷取可安全保存的錯誤碼與停用原因，不把 HTTP URL 或通知本文帶到送達紀錄。
fn send_failure(response: Option<&SendMessageResponse>) -> crate::core::alert::SendFailure {
    let code = response.and_then(|r| r.error_code);
    let disabled_reason = match code {
        Some(403) => Some("blocked_by_user"),
        Some(400)
            if response
                .and_then(|r| r.description.as_deref())
                .is_some_and(|description| {
                    description.to_ascii_lowercase().contains("chat not found")
                }) =>
        {
            Some("chat_not_found")
        }
        _ => None,
    };
    crate::core::alert::SendFailure {
        error_code: code,
        disabled_reason,
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
async fn send(msg: &str) {
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

/// 發送到指定聊天室（依訂閱分送的收件者，7.6）；長訊息與 [`send`] 一樣切段。
async fn send_to_chat(chat_id: i64, msg: &str) {
    if msg.trim().is_empty() {
        return;
    }
    let client = get_client();
    for chunk in split_message_into_chunks(msg, MAX_MESSAGE_LEN) {
        // 不記錄訊息內容：持股內容只屬於收件者。
        if let Err(error) = client.send_to(&[chat_id], &chunk).await {
            tracing::error!("Failed to send a routed message to chat {chat_id}: {error:}");
        }
    }
}

/// 發送完整訊息並回報結果；任一切段失敗即停止，避免繼續送出不完整內容。
async fn send_to_chat_checked(
    chat_id: i64,
    message: &str,
) -> Result<(), crate::core::alert::SendFailure> {
    if message.trim().is_empty() {
        return Ok(());
    }
    for chunk in split_message_into_chunks(message, MAX_MESSAGE_LEN) {
        get_client().send_checked(chat_id, &chunk).await?;
    }
    Ok(())
}

/// 警報格式集中組裝，尚未切換主題時與既有訊息文字一致。
fn format_alert_message(alert_title: &str, details: &str) -> String {
    format!(
        "⚠️ *【系統關鍵警報】*\n*標題*︰{}\n*時間*︰{}\n*詳情*︰\n```\n{}\n```",
        text::escape_markdown_v2(alert_title),
        text::escape_markdown_v2(
            TaipeiDateTime::now()
                .strftime("%Y-%m-%d %H:%M:%S")
                .to_string()
        ),
        text::escape_markdown_v2(details)
    )
}

/// 把 Telegram 通知接上 `core::alert::AlertSink` port 的 adapter。
///
/// 依 DDD 分層，core/infra 不應該直接呼叫 interfaces（內層依賴外層的
/// 反向耦合）。因此 core 只定義抽象的 [`crate::core::alert::AlertSink`]，
/// 由這個 adapter 提供 Telegram 實作，並在 `main` 啟動時註冊——
/// core/infra 呼叫 `core::alert::notify_*` 時，訊息才會實際送到 Telegram。
pub struct TelegramAlertSink;

#[async_trait::async_trait]
impl crate::core::alert::AlertSink for TelegramAlertSink {
    async fn send_alert(&self, topic: crate::core::alert::Topic, title: &str, message: &str) {
        tracing::debug!(topic = topic.code(), "telegram alert");
        let message = format_alert_message(title, message);
        crate::app::notify_common::notify_common(topic, &message).await;
    }

    async fn send_message(&self, topic: crate::core::alert::Topic, message: &str) {
        tracing::debug!(topic = topic.code(), "telegram message");
        crate::app::notify_common::notify_common(topic, message).await;
    }

    async fn send_to_chat(&self, topic: crate::core::alert::Topic, chat_id: i64, message: &str) {
        tracing::debug!(topic = topic.code(), chat_id, "telegram routed message");
        send_to_chat(chat_id, message).await;
    }

    async fn send_to_chat_checked(
        &self,
        _topic: crate::core::alert::Topic,
        chat_id: i64,
        message: &str,
    ) -> Result<(), crate::core::alert::SendFailure> {
        send_to_chat_checked(chat_id, message).await
    }

    async fn send_legacy(&self, _topic: crate::core::alert::Topic, message: &str) {
        send(message).await;
    }
}

#[cfg(test)]
mod tests;
