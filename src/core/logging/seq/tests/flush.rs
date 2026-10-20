//! 以假 Seq 伺服器驗證批次送出、API key、錯誤容忍與整批／餘數的送出時機。

use super::*;

/// 本機假 Seq：記錄每次請求的 API key 與內文，回應指定狀態碼。
struct FakeSeq {
    endpoint: String,
    requests: Requests,
}

async fn fake_seq(status: u16) -> FakeSeq {
    use axum::{extract::State, http::HeaderMap, routing::post};

    let requests: Requests = Default::default();
    let app =
        axum::Router::new()
            .route(
                "/api/events/raw",
                post(
                    move |State(requests): State<Requests>,
                          headers: HeaderMap,
                          body: String| async move {
                        let key = headers
                            .get("X-Seq-ApiKey")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_owned);
                        requests.lock().expect("測試鎖").push((key, body));
                        axum::http::StatusCode::from_u16(status).expect("合法狀態碼")
                    },
                ),
            )
            .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("綁定本機埠");
    let endpoint = format!("http://{}", listener.local_addr().expect("本機位址"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    FakeSeq { endpoint, requests }
}

fn event(message: &str) -> SeqEvent {
    SeqEvent {
        timestamp: "2026-10-13T00:00:00.000000Z".to_string(),
        message_template: message.to_string(),
        level: SeqLogLevel::Warn.as_seq_level(),
        service: SEQ_SERVICE_NAME,
        rust_log_level: SeqLogLevel::Warn.as_rust_level(),
        logger: "test".to_string(),
        fields: HashMap::new(),
    }
}

fn client() -> Client {
    ensure_rustls_crypto_provider();
    Client::builder().build().expect("建立 HTTP client")
}

/// 批次以 CLEF（一行一筆）送出並帶 API key；送出後清空緩衝區。
#[tokio::test]
async fn flush_sends_clef_lines_with_api_key() {
    let seq = fake_seq(201).await;
    let mut buf = vec![event("第一筆"), event("第二筆")];

    flush_seq_events(&client(), &seq.endpoint, Some("secret"), &mut buf).await;

    assert!(buf.is_empty());
    let requests = seq.requests.lock().expect("測試鎖").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0.as_deref(), Some("secret"));
    let lines: Vec<serde_json::Value> = requests[0]
        .1
        .lines()
        .map(|line| serde_json::from_str(line).expect("每行都是 JSON"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["@mt"], "第二筆");
    assert_eq!(lines[1]["@l"], "Warning");
}

/// 沒有 API key 時不帶標頭；Seq 回錯誤狀態、連線失敗與空緩衝區都不會 panic。
#[tokio::test]
async fn flush_tolerates_errors_and_empty_buffers() {
    let seq = fake_seq(500).await;
    let mut buf = vec![event("被拒收")];
    flush_seq_events(&client(), &seq.endpoint, None, &mut buf).await;
    assert!(buf.is_empty());
    let requests = seq.requests.lock().expect("測試鎖").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, None);

    flush_seq_events(&client(), &seq.endpoint, None, &mut Vec::new()).await;
    assert_eq!(seq.requests.lock().expect("測試鎖").len(), 1);

    let mut unreachable = vec![event("連不上")];
    flush_seq_events(&client(), "http://127.0.0.1:1", None, &mut unreachable).await;
    assert!(unreachable.is_empty());
}

/// 背景迴圈：滿一批立即送出，sender 關閉時送出剩餘事件後結束；全部事件都送達。
#[tokio::test]
async fn process_events_flushes_full_batches_and_remainder() {
    let seq = fake_seq(201).await;
    let (tx, mut rx) = mpsc::channel(SEQ_BATCH_EVENT_LIMIT * 2);
    for i in 0..=SEQ_BATCH_EVENT_LIMIT {
        tx.send(event(&format!("事件 {i}")))
            .await
            .expect("送入佇列");
    }
    drop(tx);

    process_seq_events(&client(), &seq.endpoint, Some("secret"), &mut rx).await;

    let requests = seq.requests.lock().expect("測試鎖").clone();
    assert!(requests.len() >= 2, "{}", requests.len());
    let total: usize = requests.iter().map(|(_, body)| body.lines().count()).sum();
    assert_eq!(total, SEQ_BATCH_EVENT_LIMIT + 1);
    assert!(
        requests
            .iter()
            .all(|(_, body)| body.lines().count() <= SEQ_BATCH_EVENT_LIMIT)
    );
}
