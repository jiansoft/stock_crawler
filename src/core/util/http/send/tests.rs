use super::*;

/// 同網域的 403 在冷卻期內只告警一次，不同網域彼此不受影響。
/// 測試用的 reqwest 用戶端。
///
/// 專案的 reqwest 不帶預設 crypto provider，建 `Client` 前必須先安裝；
/// 單獨執行某個測試時（CI 逐一重跑失敗測試）沒有其他測試先幫忙裝好。
fn test_client() -> Client {
    crate::core::util::ensure_rustls_crypto_provider();
    Client::new()
}

/// 只回一次 429 的本機 HTTP 伺服器，回傳網址。
async fn spawn_rate_limited_server() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
        }
    });
    format!("http://{addr}/quote")
}

/// 快速失敗模式只在 scope 內生效。
#[tokio::test]
async fn fail_fast_flag_is_scoped() {
    assert!(!is_fail_fast());
    assert!(fail_fast(async { is_fail_fast() }).await);
    assert!(!is_fail_fast());
}

/// 快速失敗模式遇到 429 直接回錯誤，不等 5 秒以上的 backoff。
#[tokio::test]
async fn fail_fast_returns_on_rate_limit_without_backoff() {
    let url = spawn_rate_limited_server().await;
    let client = test_client();

    let started = Instant::now();
    let result = fail_fast(send_with_client(
        &client,
        Method::GET,
        &url,
        None,
        None::<fn(RequestBuilder) -> RequestBuilder>,
        None,
    ))
    .await;

    let err = result.expect_err("429 應回錯誤");
    assert!(err.to_string().contains("429"), "{err:#}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "不應等待 backoff"
    );
}

/// 快速失敗模式連線失敗時不重試，不會出現 2/4/8 秒的 backoff。
#[tokio::test]
async fn fail_fast_does_not_retry_network_errors() {
    // 先綁再放掉，取得一個目前沒有人監聽的本機埠。
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("reserve local port");
    let url = format!("http://{addr}/quote");
    let client = test_client();

    let started = Instant::now();
    let result = fail_fast(send_with_client(
        &client,
        Method::GET,
        &url,
        None,
        None::<fn(RequestBuilder) -> RequestBuilder>,
        None,
    ))
    .await;

    // Windows 對本機被拒的連線約 2 秒才回報，所以不以單次耗時判斷；
    // 一般模式會重試 3 次並在中間睡 2＋4 秒，錯誤訊息也會帶重試次數。
    let err = result.expect_err("連線失敗應回錯誤");
    assert!(!err.to_string().contains("network retries"), "{err:#}");
    assert!(started.elapsed() < Duration::from_secs(5), "不應重試");
}

#[test]
fn should_alert_forbidden_throttles_per_host() {
    let host = "should-alert-forbidden-test.example";
    let other = "should-alert-forbidden-other.example";

    assert!(should_alert_forbidden(&format!("https://{host}/a")));
    assert!(!should_alert_forbidden(&format!("https://{host}/b")));
    assert!(should_alert_forbidden(&format!("https://{other}/a")));
}

#[test]
fn test_rate_limit_backoff_returns_increasing_base_delays() {
    let d1 = rate_limit_backoff(1);
    let d2 = rate_limit_backoff(2);
    let d3 = rate_limit_backoff(3);
    let d_high = rate_limit_backoff(99);

    // Base 5000ms + jitter 0..1999ms
    assert!(d1.as_millis() >= 5_000);
    assert!(d1.as_millis() < 7_000);

    // Base 15000ms + jitter 0..1999ms
    assert!(d2.as_millis() >= 15_000);
    assert!(d2.as_millis() < 17_000);

    // Base 30000ms + jitter 0..1999ms for attempt >= 3
    assert!(d3.as_millis() >= 30_000);
    assert!(d3.as_millis() < 32_000);

    assert!(d_high.as_millis() >= 30_000);
    assert!(d_high.as_millis() < 32_000);
}

#[test]
fn test_format_form_params_log_sorts_alphabetically() {
    let mut params = HashMap::new();
    params.insert("zebra", "z");
    params.insert("apple", "a");
    params.insert("mango", "m");

    let result = format_form_params_log(&params);
    assert_eq!(result, "params=[apple=a, mango=m, zebra=z]");
}

#[test]
fn test_format_form_params_log_empty_map() {
    let params = HashMap::new();
    let result = format_form_params_log(&params);
    assert_eq!(result, "params=[]");
}

/// 只有快速失敗模式的 5xx 降為 INFO；4xx 與一般模式的 5xx 仍記 WARN。
#[test]
fn downgrade_error_status_only_for_fail_fast_server_errors() {
    use reqwest::StatusCode;
    assert!(downgrade_error_status(StatusCode::BAD_GATEWAY, true));
    assert!(downgrade_error_status(
        StatusCode::INTERNAL_SERVER_ERROR,
        true
    ));
    assert!(!downgrade_error_status(StatusCode::BAD_GATEWAY, false));
    assert!(!downgrade_error_status(StatusCode::NOT_FOUND, true));
    assert!(!downgrade_error_status(StatusCode::FORBIDDEN, false));
}

/// 降級只影響日誌層級：快速失敗模式下 5xx 回應仍原樣交給呼叫端判斷。
#[tokio::test]
async fn fail_fast_server_error_still_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
        }
    });
    let url = format!("http://{addr}/quote");
    let client = test_client();
    let response = fail_fast(send_with_client(
        &client,
        Method::GET,
        &url,
        None,
        None::<fn(RequestBuilder) -> RequestBuilder>,
        None,
    ))
    .await
    .expect("5xx 不是傳輸錯誤");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
}
