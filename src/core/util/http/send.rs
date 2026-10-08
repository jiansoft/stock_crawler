//! HTTP 請求的送出與重試：網路層與 429 的雙層重試、並發上限、403 告警與請求日誌。

use std::{
    collections::HashMap,
    future::Future,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use reqwest::{Client, Method, RequestBuilder, Response, header};
use tokio::sync::Semaphore;

use super::redact::{redact_secrets, redact_url};
use super::{get_client, user_agent};

/// A semaphore for limiting concurrent requests.
///
/// 限制最多 5 個並發請求，避免被目標網站封禁。
static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(5));

/// 網路傳輸失敗（TCP 層）的最大重試次數。
const MAX_NETWORK_RETRIES: u32 = 3;
/// HTTP 429 Too Many Requests 的最大重試次數。
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
/// 同一個網域重複發送 403 告警之間的最短間隔。
///
/// 來源站台一旦掛上 WAF，之後每次請求都會是 403；若每次都告警，真正需要注意的事件
/// 會被洗掉。同網域在這個間隔內只會發一次，其餘只留 log。
const FORBIDDEN_ALERT_COOLDOWN: Duration = Duration::from_secs(60 * 60 * 12);
/// 成功回應耗時達到此門檻（毫秒）時改以 WARN 記錄。
///
/// 正式機平常 p99 約 0.4 秒、最慢約 2.4 秒，請求逾時為 15 秒；5 秒代表來源站明顯變慢。
const SLOW_REQUEST_WARN_MS: u64 = 5_000;

/// 快速失敗模式下單次請求的逾時。
///
/// 一般請求沿用用戶端的 15 秒逾時；即時報價有其他站可以換，等太久反而拖慢整輪輪詢。
/// 2026-10-07 正式機各站 p99 約 4.2～5.6 秒（PcHome 9.6 秒），6 秒只截掉明顯異常的尾巴。
const FAIL_FAST_TIMEOUT: Duration = Duration::from_secs(6);

tokio::task_local! {
    /// 存在時代表目前的請求處於快速失敗模式，見 [`fail_fast`]。
    static FAIL_FAST: ();
}

/// 在快速失敗模式下執行 `fut`：其中的 HTTP 請求不做網路重試與 429 重試，
/// 單次逾時縮短為 [`FAIL_FAST_TIMEOUT`]，失敗只記 WARN。
///
/// 給「有其他來源可以換」的呼叫端使用（即時報價站點池）。一般批次請求的重試是為了
/// 撐過來源站的短暫異常，但站點池換下一站就好；網路重試的 2/4/8 秒 backoff 會讓
/// 單一請求拖到 15 秒以上，2026-10-07 PcHome p99 9.6 秒、`http.failed` 12 筆即為此。
pub async fn fail_fast<F: Future>(fut: F) -> F::Output {
    FAIL_FAST.scope((), fut).await
}

/// 目前是否處於快速失敗模式。
fn is_fail_fast() -> bool {
    FAIL_FAST.try_with(|_| ()).is_ok()
}

/// 各網域最近一次發送 403 告警的時間。
///
/// 只存在於行程記憶體：重啟後重新告警一次是可接受的，換得 core 層不必依賴外部儲存。
static FORBIDDEN_ALERTED_AT: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 判斷這個網址的 403 是否該送出告警，並在送出時記下時間。
///
/// 取不到網域（網址無法解析）時一律告警，寧可多通知也不要漏掉。
fn should_alert_forbidden(url: &str) -> bool {
    let Some(host) = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .map(str::to_string)
    else {
        return true;
    };

    let Ok(mut alerted_at) = FORBIDDEN_ALERTED_AT.lock() else {
        return true;
    };

    let now = Instant::now();
    if let Some(last) = alerted_at.get(&host)
        && now.duration_since(*last) < FORBIDDEN_ALERT_COOLDOWN
    {
        return false;
    }

    alerted_at.insert(host, now);
    true
}

/// 以指定方法、URL、headers、body 發送 HTTP 請求，含雙層重試：
/// - **網路層**（TCP 失敗）：最多 `MAX_NETWORK_RETRIES` 次，2^n 秒 backoff。
/// - **頻率限制**（HTTP 429）：最多 `MAX_RATE_LIMIT_RETRIES` 次，5/15/30s + 最多 2s jitter。
///
/// 在 [`fail_fast`] 範圍內兩層都不重試，單次逾時改為 [`FAIL_FAST_TIMEOUT`]。
pub(super) async fn send(
    method: Method,
    url: &str,
    headers: Option<header::HeaderMap>,
    body: Option<impl FnOnce(RequestBuilder) -> RequestBuilder>,
    request_detail: Option<String>,
) -> Result<Response> {
    let client = get_client()?;
    send_with_client(client, method, url, headers, body, request_detail).await
}

/// 使用指定用戶端發送請求，供共用 HTTP 入口與測試使用，並處理重試及 403 告警。
/// 一般回應記為 DEBUG；慢回應、HTTP 錯誤與待重試的網路錯誤／429 記為 WARN，重試耗盡記為 ERROR。
/// 回傳尚未讀取內容的回應；HTTP 狀態碼仍交由呼叫端判斷，網路或限流重試耗盡則回傳錯誤。
pub(super) async fn send_with_client(
    client: &Client,
    method: Method,
    url: &str,
    headers: Option<header::HeaderMap>,
    body: Option<impl FnOnce(RequestBuilder) -> RequestBuilder>,
    request_detail: Option<String>,
) -> Result<Response> {
    let request_detail_suffix = request_detail
        .as_deref()
        .map(|d| format!(" {d}"))
        .unwrap_or_default();

    // ── G2: per-request User-Agent 輪轉 ────────────────────────────────────
    // 每次請求重新產生 UA，避免長時間使用固定 UA 被目標站辨識封鎖。
    // 若呼叫端在 headers 中已設定 User-Agent，.headers(h) 會在後面覆蓋，
    // 讓呼叫端的自訂 UA 優先。
    let mut rb = client
        .request(method.clone(), url)
        .header(header::USER_AGENT, user_agent::gen_random_ua());

    if let Some(h) = headers {
        rb = rb.headers(h);
    }
    if let Some(body_fn) = body {
        rb = body_fn(rb);
    }

    // ── G1: 雙層重試計數器 ─────────────────────────────────────────────────
    let mut network_attempt = 0u32;
    let mut rate_limit_attempt = 0u32;
    let fail_fast = is_fail_fast();
    let (max_network_attempts, max_rate_limit_retries) = if fail_fast {
        (1, 0)
    } else {
        (MAX_NETWORK_RETRIES, MAX_RATE_LIMIT_RETRIES)
    };
    if fail_fast {
        rb = rb.timeout(FAIL_FAST_TIMEOUT);
    }

    loop {
        // 複製 RequestBuilder 以供重試使用。
        // 如果複製失敗（例如 RequestBuilder 中包含串流），則將 URL 脫敏後回傳錯誤，避免洩漏 Token。
        let rb_clone = rb
            .try_clone()
            .ok_or_else(|| anyhow!("Failed to clone RequestBuilder for {}", redact_url(url)))?;

        let (res, elapsed_ms) = {
            let _permit = SEMAPHORE.acquire().await;
            let start = Instant::now();
            let res = rb_clone.send().await;
            (res, start.elapsed().as_millis() as u64)
        };

        match res {
            Ok(response) => {
                let status = response.status();
                // 收到回應不代表 HTTP 成功；先遮罩 URL，再依狀態碼決定層級，避免正常爬取淹沒警告。
                let safe_url = redact_url(url);
                // 會進入下方重試流程的 429，由 http.rate_limited／http.rate_limit_exhausted 記錄，
                // 這裡不再重複記一筆。
                let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    && !url.contains("api.telegram.org");

                if status.is_client_error() || status.is_server_error() {
                    if !rate_limited {
                        tracing::warn!(
                            url = %safe_url,
                            method = method.as_str(),
                            status = status.as_u16(),
                            elapsed_ms,
                            "http.done{request_detail_suffix}"
                        );
                    }
                } else if elapsed_ms >= SLOW_REQUEST_WARN_MS {
                    // 成功回應平常只記 DEBUG；來源站明顯變慢時仍要在檔案日誌留下痕跡。
                    tracing::warn!(
                        url = %safe_url,
                        method = method.as_str(),
                        status = status.as_u16(),
                        elapsed_ms,
                        "http.slow{request_detail_suffix}"
                    );
                } else {
                    tracing::debug!(
                        url = %safe_url,
                        method = method.as_str(),
                        status = status.as_u16(),
                        elapsed_ms,
                        "http.done{request_detail_suffix}"
                    );
                }

                // ── 429 Too Many Requests：exponential backoff retry ───────
                if rate_limited {
                    if fail_fast {
                        // 快速失敗模式由呼叫端換下一個來源，不在這裡等 backoff。
                        tracing::warn!(
                            url = %safe_url,
                            method = method.as_str(),
                            status = status.as_u16(),
                            elapsed_ms,
                            "http.rate_limited.fail_fast{request_detail_suffix}"
                        );
                        return Err(anyhow!("Rate limited (429) at {}", safe_url));
                    }
                    rate_limit_attempt += 1;
                    if rate_limit_attempt <= max_rate_limit_retries {
                        let delay = rate_limit_backoff(rate_limit_attempt);
                        tracing::warn!(
                            url = %safe_url,
                            attempt = rate_limit_attempt,
                            delay_ms = delay.as_millis() as u64,
                            "http.rate_limited"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    tracing::error!(
                        url = %safe_url,
                        method = method.as_str(),
                        status = status.as_u16(),
                        attempt = rate_limit_attempt,
                        elapsed_ms,
                        "http.rate_limit_exhausted{request_detail_suffix}"
                    );
                    return Err(anyhow!(
                        "Rate limited (429) at {} after {rate_limit_attempt} retries",
                        safe_url
                    ));
                }

                // ── 403 Forbidden：發送系統告警，不重試 ──────────────────
                // 透過 core::alert 抽象介面發送（實際管道由 main 註冊的 adapter 決定），
                // core 層不再直接依賴 interfaces::bot（反向耦合已移除）。
                // 冷卻期內不再告警；上方 http.done 已以 WARN 記下這次 403，不必再補一筆。
                if status == reqwest::StatusCode::FORBIDDEN
                    && !url.contains("api.telegram.org")
                    && should_alert_forbidden(url)
                {
                    let alert_url = url.to_string();
                    tokio::spawn(async move {
                        crate::core::alert::send_alert(
                            "爬蟲遭遇 IP 阻擋 (403)",
                            &format!("請求網址: {alert_url}"),
                        )
                        .await;
                    });
                }

                return Ok(response);
            }
            Err(why) => {
                network_attempt += 1;
                // reqwest::Error 的 Debug 會把**完整的請求 URL**（含 Telegram Token）
                // 一併印出來，繞過下方 safe_url 的遮蔽。err_str 同時進到 tracing 的
                // error 欄位與往上拋的 anyhow 訊息，兩條路徑最後都會落到日誌檔，
                // 因此必須在這裡就先遮蔽。
                let err_str = redact_secrets(&format!("{why:?}"));
                let safe_url = redact_url(url);
                // 快速失敗模式不重試；呼叫端會改用其他來源，所以只記 WARN。
                if fail_fast {
                    tracing::warn!(
                        url = %safe_url,
                        attempt = network_attempt,
                        error = %err_str,
                        elapsed_ms,
                        "http.failed.fail_fast{request_detail_suffix}"
                    );
                    return Err(anyhow!("Failed to send {}: {err_str}", safe_url));
                }

                // 尚可重試的暫時錯誤使用 WARN，只有耗盡次數才以 ERROR 標示請求最終失敗。
                if network_attempt >= max_network_attempts {
                    tracing::error!(
                        url = %safe_url,
                        attempt = network_attempt,
                        error = %err_str,
                        elapsed_ms,
                        "http.failed{request_detail_suffix}"
                    );
                    return Err(anyhow!(
                        "Failed to send {} after {network_attempt} network retries; \
                         last error: {err_str}",
                        safe_url
                    ));
                }

                tracing::warn!(
                    url = %safe_url,
                    attempt = network_attempt,
                    error = %err_str,
                    elapsed_ms,
                    "http.failed{request_detail_suffix}"
                );

                // 2^n 秒 backoff：1→2s、2→4s、3→8s
                tokio::time::sleep(Duration::from_secs(2u64.pow(network_attempt))).await;
            }
        }
    }
}

/// 429 Too Many Requests 的 backoff 策略：5s / 15s / 30s，附加最多 2s jitter。
fn rate_limit_backoff(attempt: u32) -> Duration {
    let base_ms: u64 = match attempt {
        1 => 5_000,
        2 => 15_000,
        _ => 30_000,
    };
    let jitter_ms = rand::random::<u64>() % 2_000;
    Duration::from_millis(base_ms + jitter_ms)
}

/// 把表單參數排序後組成日誌用字串（`params=[a=1, b=2]`）。
pub(super) fn format_form_params_log(params: &HashMap<&str, &str>) -> String {
    let mut entries = params
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    entries.sort();

    format!("params=[{}]", entries.join(", "))
}

#[cfg(test)]
mod tests {
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
}
