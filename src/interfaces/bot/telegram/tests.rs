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
/// `LAST_SENT_AT` 是程序層級的全域狀態，送信流程測試也會寫入（真實時鐘），
/// 因此先取得 [`SEND_SLOT_TEST_LOCK`] 並清掉上次送出時間。
#[tokio::test(start_paused = true)]
async fn test_acquire_send_slot_enforces_min_interval() {
    let _serial = SEND_SLOT_TEST_LOCK.lock().await;
    *LAST_SENT_AT.lock().await = None;
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

/// 會經過 [`acquire_send_slot`] 的測試共用這把鎖，避免並行時互相改到 `LAST_SENT_AT`。
static SEND_SLOT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 本機假 Telegram API：依序回應預先排好的 `(HTTP 狀態, JSON)`，並記錄每次請求。
///
/// 路徑含 `api.telegram.org`，讓共用 HTTP 層與正式環境一樣不對 429 做自己的重試。
struct FakeTelegram {
    client: Telegram,
    requests: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
}

async fn fake_telegram(recipients: Vec<i64>, replies: Vec<(u16, &str)>) -> FakeTelegram {
    use axum::{Json, extract::State, http::StatusCode, routing::post};
    use std::sync::{Arc, Mutex};
    type Shared = (
        Arc<Mutex<Vec<serde_json::Value>>>,
        Arc<Mutex<std::collections::VecDeque<(u16, String)>>>,
    );

    let requests: Arc<Mutex<Vec<serde_json::Value>>> = Default::default();
    let replies = Arc::new(Mutex::new(
        replies
            .into_iter()
            .map(|(status, body)| (status, body.to_owned()))
            .collect::<std::collections::VecDeque<_>>(),
    ));
    let app =
        axum::Router::new()
            .route(
                "/api.telegram.org/sendMessage",
                post(
                    |State((requests, replies)): State<Shared>,
                     Json(body): Json<serde_json::Value>| async move {
                        requests.lock().expect("測試鎖").push(body);
                        let (status, reply) = replies
                            .lock()
                            .expect("測試鎖")
                            .pop_front()
                            .unwrap_or((200, OK_REPLY.to_owned()));
                        (
                            StatusCode::from_u16(status).expect("合法狀態碼"),
                            [("content-type", "application/json")],
                            reply,
                        )
                    },
                ),
            )
            .with_state((requests.clone(), replies));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("綁定本機埠");
    let address = listener.local_addr().expect("本機位址");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    FakeTelegram {
        client: Telegram {
            send_message_url: format!("http://{address}/api.telegram.org/sendMessage"),
            recipients,
        },
        requests,
    }
}

const OK_REPLY: &str = r#"{"ok":true,"result":{"message_id":1}}"#;
const BAD_REQUEST: &str =
    r#"{"ok":false,"error_code":400,"description":"Bad Request: can't parse entities"}"#;
const RATE_LIMITED: &str = r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":0}}"#;

impl FakeTelegram {
    /// 依序取出每次請求的 `(chat_id, parse_mode, text)`。
    fn calls(&self) -> Vec<(i64, String, String)> {
        self.requests
            .lock()
            .expect("測試鎖")
            .iter()
            .map(|body| {
                (
                    body["chat_id"].as_i64().unwrap_or_default(),
                    body["parse_mode"].as_str().unwrap_or_default().to_owned(),
                    body["text"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }
}

/// 送給每位接收者；只要有一位成功就回傳成功，其餘失敗只記錄。
#[tokio::test]
async fn send_broadcasts_to_every_recipient() {
    let _serial = SEND_SLOT_TEST_LOCK.lock().await;
    let fake = fake_telegram(vec![1, 2], vec![(200, OK_REPLY), (400, BAD_REQUEST)]).await;
    let response = fake.client.send("通知").await.expect("至少一位送達");
    assert!(response.ok);
    let chats: Vec<i64> = fake.calls().iter().map(|call| call.0).collect();
    assert_eq!(chats.len(), 2);
    assert!(chats.contains(&1) && chats.contains(&2));
}

/// 429 依 retry_after 等待後以 MarkdownV2 原樣重送。
#[tokio::test]
async fn send_retries_once_after_rate_limit() {
    let _serial = SEND_SLOT_TEST_LOCK.lock().await;
    let fake = fake_telegram(vec![1], vec![(429, RATE_LIMITED), (200, OK_REPLY)]).await;
    fake.client.send("*粗體*").await.expect("重送成功");
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|call| call.1 == MARKDOWN_V2 && call.2 == "*粗體*")
    );
}

/// MarkdownV2 解析失敗時移除跳脫反斜線，改以純文字重送。
#[tokio::test]
async fn send_falls_back_to_plain_text() {
    let _serial = SEND_SLOT_TEST_LOCK.lock().await;
    let fake = fake_telegram(vec![1], vec![(400, BAD_REQUEST), (200, OK_REPLY)]).await;
    fake.client.send(r"價格 12\.5").await.expect("純文字送達");
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].1, PLAIN_TEXT);
    assert_eq!(calls[1].2, "價格 12.5");
}

/// 429 重送仍失敗、純文字也失敗時回傳錯誤；每種補救只做一次。
#[tokio::test]
async fn send_gives_up_after_each_remedy_once() {
    let _serial = SEND_SLOT_TEST_LOCK.lock().await;
    let fake = fake_telegram(
        vec![1],
        vec![(429, RATE_LIMITED), (429, RATE_LIMITED), (400, BAD_REQUEST)],
    )
    .await;
    let error = fake.client.send("通知").await.expect_err("全部失敗");
    assert!(error.to_string().contains("plain-text fallback"), "{error}");
    let modes: Vec<String> = fake.calls().into_iter().map(|call| call.1).collect();
    assert_eq!(modes, [MARKDOWN_V2, MARKDOWN_V2, PLAIN_TEXT]);
}
