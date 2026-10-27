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

/// 開啟 DEBUG 層級的訂閱者，讓快速失敗模式降級後的日誌欄位實際被求值（也確認它不會 panic）。
fn debug_subscriber() -> tracing::subscriber::DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .finish();
    tracing::subscriber::set_default(subscriber)
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

/// 彙總模式只改逐筆日誌，不會開啟快速失敗或影響外層 scope。
#[tokio::test]
async fn summarized_preserves_normal_retry_policy() {
    assert!(SUMMARIZED.try_with(|_| ()).is_err());
    summarized(async {
        assert!(!is_fail_fast());
        assert!(SUMMARIZED.try_with(|_| ()).is_ok());
        let _guard = debug_subscriber();
        let client = test_client();
        let error = send_with_client(
            &client,
            Method::GET,
            "http://[",
            None,
            None::<fn(RequestBuilder) -> RequestBuilder>,
            None,
        )
        .await
        .expect_err("無效 URL 應回錯誤");
        assert!(error.to_string().contains("after 3 network retries"));
    })
    .await;
    assert!(SUMMARIZED.try_with(|_| ()).is_err());
}

/// 快速失敗模式遇到 429 直接回錯誤，不等 5 秒以上的 backoff。
#[tokio::test]
async fn fail_fast_returns_on_rate_limit_without_backoff() {
    let _guard = debug_subscriber();
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
    let _guard = debug_subscriber();
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

/// 降級只影響日誌層級：快速失敗模式下 5xx 回應仍原樣交給呼叫端判斷。
#[tokio::test]
async fn fail_fast_server_error_still_returns_response() {
    let _guard = debug_subscriber();
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

/// 一般模式的 5xx 沒有備援，仍走 WARN 分支，回應同樣原樣交給呼叫端。
#[tokio::test]
async fn normal_mode_server_error_still_warns_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
        }
    });
    let url = format!("http://{addr}/quote");
    let client = test_client();
    let response = send_with_client(
        &client,
        Method::GET,
        &url,
        None,
        None::<fn(RequestBuilder) -> RequestBuilder>,
        None,
    )
    .await
    .expect("5xx 不是傳輸錯誤");
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
}
