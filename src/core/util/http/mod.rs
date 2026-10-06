//! # 共用 HTTP 客戶端
//!
//! 對外的請求函式（[`get`]、[`get_json`]、[`post`]、[`post_use_json`] 等）都在這裡；
//! 送出與重試在 `send` 子模組，敏感資訊遮罩在 `redact` 子模組。

use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use reqwest::{Client, Method, RequestBuilder, Response, header, header::SET_COOKIE};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::core::util;

/// HTML 解析輔助工具。
pub mod element;
/// 日誌與錯誤訊息的敏感資訊遮罩。
mod redact;
/// 請求的送出與重試。
mod send;
/// 隨機 User-Agent 產生器。
pub mod user_agent;

pub use redact::{redact_secrets, redact_url};
use send::{format_form_params_log, send, send_with_client};

/// A singleton instance of the reqwest client.
static CLIENT: OnceCell<Client> = OnceCell::new();

/// HTTP 回應 body 的大小上限（8 MiB）。
///
/// 正常來源（TWSE/TPEx JSON、財經網站頁面）最大約 1～2 MiB，8 MiB 已留足
/// 餘裕；異常或惡意的 upstream 若回傳超大 body，舊版會全部讀進記憶體，
/// 可能造成高記憶體使用甚至 OOM。超限時中止讀取並回傳錯誤。
///
/// 已知會超過的全市場資料（例如董監持股明細約 10 MiB）改用
/// [`get_json_with_limit`] 個別放寬。
const MAX_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;

/// 以「逐塊讀取 + 大小上限」讀取 HTTP 回應 body。
///
/// 讀取策略（保母級說明）：
/// 1. 若回應有 `Content-Length` 標頭，先檢查宣告大小——超限就連第一個
///    位元組都不讀，直接回報錯誤。
/// 2. 沒有或通過宣告檢查後，改用 `Response::chunk()` 一塊一塊地讀；
///    每讀一塊就累計大小，超過上限立即中止（`Content-Length` 可以造假
///    或缺席，所以實際讀取仍要逐塊把關）。
///
/// 這取代了 `res.bytes()` / `res.text()` 這類「一次全部讀進記憶體、
/// 沒有上限」的讀法。
async fn read_limited_body(mut res: Response, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    // 錯誤訊息中的 URL 一律先脫敏，避免把 Telegram token 等敏感片段寫進 log。
    let safe_url = redact_url(url);

    // 第一道：宣告大小檢查（若 upstream 有提供）。
    if let Some(content_length) = res.content_length()
        && content_length > max_bytes as u64
    {
        bail!(
            "response body too large for {safe_url}: content-length {content_length} bytes exceeds limit {max_bytes}"
        );
    }

    // 第二道：實際逐塊累積，逐塊檢查。chunk() 回傳 Ok(None) 代表 body 讀完。
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .with_context(|| format!("Error reading response body from {safe_url}"))?
    {
        if body.len() + chunk.len() > max_bytes {
            bail!("response body too large for {safe_url}: exceeds limit {max_bytes} bytes");
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

#[derive(Serialize, Deserialize)]
/// An empty struct to represent an empty request or response.
pub struct Empty {}

/// An asynchronous trait that provides a method to force convert a reqwest::Response body
/// from Big5 encoding to UTF-8 encoding.
#[async_trait]
pub trait TextForceBig5 {
    /// Converts the body of a reqwest::Response from Big5 encoding to UTF-8 encoding.
    ///
    /// This method awaits the bytes of the response, converts them to a Vec<u8>,
    /// and then calls `big5_2_utf8` function to perform the encoding conversion.
    ///
    /// # Returns
    ///
    /// * `Result<String>`: A UTF-8 encoded string if the conversion is successful,
    /// or an error if the conversion fails.
    async fn text_force_big5(self) -> Result<String>;
}

/// Implements the TextForceBig5 trait for reqwest::Response.
#[async_trait]
impl TextForceBig5 for Response {
    async fn text_force_big5(mut self) -> Result<String> {
        util::text::big5_2_utf8(self.bytes().await?.as_ref())
    }
}

/// Returns the reqwest client singleton instance or creates one if it doesn't exist.
///
/// # Returns
///
/// * Result<&'static Client>: A reference to the reqwest client instance,
///   or an error if the client cannot be created.
fn get_client() -> Result<&'static Client> {
    CLIENT.get_or_try_init(|| {
        util::ensure_rustls_crypto_provider();

        Client::builder()
            // ===== 壓縮 =====
            // Accept-Encoding 是協商式的：只宣告 gzip/brotli，站方會自動改用支援的編碼。
            // 不啟用 zstd 以減少一個原生解壓依賴（Cargo.toml 已移除該 feature）。
            .brotli(true)
            .gzip(true)
            // ===== 超時設置 =====
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            // ===== TCP 優化 =====
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(60))
            // ===== HTTP/2 優化 =====
            // 注意：移除 http2_prior_knowledge() 和 http2_adaptive_window()
            // 因為某些 API（如 Telegram）對 HTTP/2 幀大小有特殊要求
            // 讓 reqwest 自動協商協議版本更安全
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            // ===== 連接池 =====
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            // ===== Cookie 和重定向 =====
            // 大部分 crawler 都是 stateless request；避免把各站點回傳的 cookie
            // 長期保留在全域 client 內，造成盤中輪詢時記憶體工作集持續增長。
            .redirect(reqwest::redirect::Policy::limited(5))
            // ===== Headers =====
            .referer(true)
            .user_agent(user_agent::gen_random_ua())
            .build()
            // context 保留 reqwest 原始錯誤在 source chain，不再壓成純文字。
            .context("Failed to create reqwest client")
    })
}

/// Performs an HTTP GET request and deserializes the JSON response into the specified type.
///
/// # Type Parameters
///
/// * `RES`: The type to deserialize the JSON response into. It must implement `DeserializeOwned`.
///
/// # Arguments
///
/// * `url`: The URL to send the GET request to.
///
/// # Returns
///
/// * `Result<RES>`: The deserialized response, or an error if the request fails or the response cannot be deserialized.
pub async fn get_json<RES: DeserializeOwned>(url: &str) -> Result<RES> {
    get_json_with_limit(url, MAX_RESPONSE_BODY_BYTES).await
}

/// 與 [`get_json`] 相同，但 body 大小上限改用 `max_body_bytes`。
///
/// 只給已知會超過預設上限的全市場資料使用（例如董監持股明細，解壓後約 10 MiB）；
/// 上限仍要設，避免異常回應把記憶體吃光。
pub async fn get_json_with_limit<RES: DeserializeOwned>(
    url: &str,
    max_body_bytes: usize,
) -> Result<RES> {
    let res = get_response(url, None).await?;
    let status = res.status();
    // 以「串流 + 大小上限」讀取 body，取代無上限的 res.bytes()。
    let res_body = read_limited_body(res, url, max_body_bytes).await?;
    let res_body_preview = String::from_utf8_lossy(res_body.as_ref());
    let safe_url = redact_url(url);

    if !status.is_success() {
        return Err(anyhow!(
            "HTTP request failed with status {} for {}. Body: {}",
            status,
            safe_url,
            util::text::truncate(&res_body_preview, 200)
        ));
    }

    // with_context 保留 serde 原始錯誤在 source chain；
    // 錯誤訊息中的 body 一律用 truncate 依字元截斷成固定長度片段。
    serde_json::from_slice(res_body.as_ref()).with_context(|| {
        format!(
            "Error parsing response JSON from {}. Body: {}",
            safe_url,
            util::text::truncate(&res_body_preview, 200)
        )
    })
}

/// 執行 HTTP GET 並回傳原始 `Response`。
///
/// 這個 helper 保留呼叫端自行處理 status code、header 與 body 的彈性，
/// 適合需要讀取 cookie、串流或非文字內容的情境。
pub async fn get_response(url: &str, headers: Option<header::HeaderMap>) -> Result<Response> {
    send(Method::GET, url, headers, None::<fn(_) -> _>, None).await
}

/// 使用指定 client 執行 HTTP GET。
///
/// 這個 helper 讓來源模組可以套用專用 transport profile，
/// 同時仍沿用共用的重試、semaphore 與 HTTP diagnostics。
pub(crate) async fn get_response_with_client(
    client: &Client,
    url: &str,
    headers: Option<header::HeaderMap>,
) -> Result<Response> {
    send_with_client(client, Method::GET, url, headers, None::<fn(_) -> _>, None).await
}

/// Performs an HTTP GET request and returns the response as text.
///
/// # Arguments
///
/// * `url`: The URL to send the GET request to.
///
/// # Returns
///
/// * `Result<String>`: The response text, or an error if the request fails or the response cannot be parsed.
pub async fn get(url: &str, headers: Option<header::HeaderMap>) -> Result<String> {
    // context 保留 reqwest 原始錯誤在 source chain。
    // 注意：text() 會依 Content-Type 的 charset 解碼，因此這裡不改用
    // read_limited_body（其為原始位元組，套 UTF-8 lossy 會弄壞非 UTF-8 頁面）。
    get_response(url, headers)
        .await?
        .text()
        .await
        .context("Error parsing response text")
}

/// 從 HTTP 回應標頭萃取 `Set-Cookie` 並串成單一 cookie 字串。
///
/// 若回應中沒有任何 `Set-Cookie`，則回傳 `None`。
pub fn extract_cookies(response: &Response) -> Option<String> {
    let cookies: Vec<String> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|val| val.to_str().ok()) // ✅ 安全處理
        .map(String::from)
        .collect();

    if cookies.is_empty() {
        None
    } else {
        Some(cookies.join("; "))
    }
}

/// Performs an HTTP GET request and returns the response as Big5 encoded text.
///
/// # Arguments
///
/// * `url`: The URL to send the GET request to.
///
/// # Returns
///
/// * `Result<String>`: The Big5 encoded response text, or an error if the request fails or the response cannot be parsed.
pub async fn get_use_big5(url: &str) -> Result<String> {
    // context 保留底層錯誤在 source chain。
    send(Method::GET, url, None, None::<fn(_) -> _>, None)
        .await?
        .text_force_big5()
        .await
        .context("Error parsing response text use BIG5")
}

/// Performs an HTTP POST request with JSON request and response, and specified headers.
///
/// # Type Parameters
///
/// * `REQ`: The request type to serialize as JSON. It must implement `Serialize`.
/// * `RES`: The response type to deserialize from JSON. It must implement `DeserializeOwned`.
///
/// # Arguments
///
/// * `url`: The URL to send the POST request to.
/// * `headers`: An optional set of headers to include with the request.
/// * `req`: An optional reference to the request object to be serialized as JSON.
///
/// # Returns
///
/// * `Result<RES>`: The deserialized response, or an error if the request fails or the response cannot be deserialized.
pub async fn post_use_json<REQ, RES>(
    url: &str,
    headers: Option<header::HeaderMap>,
    req: Option<&REQ>,
) -> Result<RES>
where
    REQ: Serialize,
    RES: DeserializeOwned,
{
    let res = send(
        Method::POST,
        url,
        headers,
        Some(
            |rb: RequestBuilder| {
                if let Some(r) = req { rb.json(r) } else { rb }
            },
        ),
        None,
    )
    .await?;

    // 以「串流 + 大小上限」讀取 body，取代無上限的 res.text()。
    let res_body = read_limited_body(res, url, MAX_RESPONSE_BODY_BYTES).await?;
    let res_body_preview = String::from_utf8_lossy(res_body.as_ref());

    // with_context 保留 serde 原始錯誤在 source chain。
    // 錯誤訊息中的 body 改為固定長度片段（舊版會把「整份 body」塞進錯誤，
    // 造成大型 log，回應含敏感內容時也可能整段被寫進日誌）。
    serde_json::from_slice(res_body.as_ref()).with_context(|| {
        format!(
            "Error parsing response JSON from {}. Body: {}",
            redact_url(url),
            util::text::truncate(&res_body_preview, 200)
        )
    })
}

/// Performs an HTTP POST request with form data and specified headers, and returns the response as text.
///
/// # Arguments
///
/// * `url`: The URL to send the POST request to.
/// * `headers`: An optional set of headers to include with the request.
/// * `params`: An optional map of form data key-value pairs.
///
/// # Returns
///
/// * `Result<String>`: The response text, or an error if the request fails
///   or the response cannot be parsed.
pub async fn post(
    url: &str,
    headers: Option<header::HeaderMap>,
    params: Option<HashMap<&str, &str>>,
) -> Result<String> {
    let body_fn: Option<fn(RequestBuilder) -> RequestBuilder> = None;
    let response = match params {
        Some(p) => {
            let request_detail = format_form_params_log(&p);
            send(
                Method::POST,
                url,
                headers,
                Some(move |rb: RequestBuilder| rb.form(&p)),
                Some(request_detail),
            )
            .await?
        }
        None => send(Method::POST, url, headers, body_fn, None).await?,
    };

    // context 保留 reqwest 原始錯誤在 source chain。
    response.text().await.context("Error parsing response text")
}

/// HTTP 層已改用 `tracing::*!()` 輸出，不再有獨立的 channel queue。
/// 保留此函式供呼叫端相容，永遠回傳零值摘要。
pub(crate) fn diagnostics_snapshot() -> crate::core::logging::LoggerRuntimeStatus {
    crate::core::logging::LoggerRuntimeStatus::default()
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use concat_string::concat_string;

    use super::*;

    /// 手動驗證外部 HTTP 來源是否可正常請求。
    ///
    /// 這個測試需要實際連線 TWSE 與 httpbin，預設測試集不應依賴外部網路，
    /// 因此標記為 ignored，避免 CI 或離線環境因網路限制而失敗。
    #[tokio::test]
    #[ignore]
    async fn test_request() {
        let url = concat_string!(
            "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
            Local::now().format("%Y%m%d").to_string(),
            "&_=",
            Local::now().timestamp_millis().to_string()
        );

        tracing::debug!("request_get:{:?}", get(&url, None).await);

        let bytes = reqwest::get("https://httpbin.org/ip")
            .await
            .unwrap()
            .bytes()
            .await;

        println!("bytes: {:#?}", bytes);
    }

    #[tokio::test]
    async fn test_get() {
        match get("https://jiansoft.mooo.com/stock/revenues", None).await {
            Ok(_) => {}
            Err(why) => {
                tracing::error!("Failed to get because {:?}", why);
            }
        }
    }
}
