use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::{Lazy, OnceCell};
use reqwest::{header, header::SET_COOKIE, Client, Method, RequestBuilder, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{logging::Logger, util};

pub mod element;
pub mod user_agent;

/// A semaphore for limiting concurrent requests.
///
/// 限制最多 5 個並發請求，避免被目標網站封禁。
static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(5));

/// A singleton instance of the reqwest client.
static CLIENT: OnceCell<Client> = OnceCell::new();

static LOGGER: Lazy<Logger> = Lazy::new(|| Logger::new("http"));

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
        Client::builder()
            // ===== 壓縮 =====
            .brotli(true)
            .deflate(true)
            .gzip(true)
            .zstd(true)
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
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            // ===== Headers =====
            .referer(true)
            .user_agent(user_agent::gen_random_ua())
            .build()
            .map_err(|e| anyhow!("Failed to create reqwest client: {:?}", e))
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
    //send(Method::GET, url, None, None::<fn(_) -> _>)
    get_response(url, None)
        .await?
        .json::<RES>()
        .await
        .map_err(|e| anyhow!("Error parsing response JSON: {:?}", e))
}

pub async fn get_response(url: &str, headers: Option<header::HeaderMap>) -> Result<Response> {
    send(Method::GET, url, headers, None::<fn(_) -> _>).await
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
    get_response(url, headers)
        .await?
        .text()
        .await
        .map_err(|e| anyhow!("Error parsing response text: {:?}", e))
}

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
    send(Method::GET, url, None, None::<fn(_) -> _>)
        .await?
        .text_force_big5()
        .await
        .map_err(|e| anyhow!("Error parsing response text use BIG5: {:?}", e))
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
                if let Some(r) = req {
                    rb.json(r)
                } else {
                    rb
                }
            },
        ),
    )
    .await?;

    /*res.json::<RES>()
    .await
    .map_err(|why| anyhow!("Error parsing response JSON: {:?}", why))*/
    let res_body = res
        .text()
        .await
        .map_err(|e| anyhow!("Error reading response body: {}", e))?;

    // Print the response body
    //println!("Response body: {}", res_body);

    serde_json::from_str(&res_body)
        .map_err(|e| anyhow!("Error parsing response JSON({}): {:?}", &res_body, e))
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
            send(
                Method::POST,
                url,
                headers,
                Some(move |rb: RequestBuilder| rb.form(&p)),
            )
            .await?
        }
        None => send(Method::POST, url, headers, body_fn).await?,
    };

    response
        .text()
        .await
        .map_err(|why| anyhow!("Error parsing response text: {:?}", why))
}

/// HTTP 請求失敗時的最大重試次數。
const MAX_RETRIES: usize = 2;

/// Sends an HTTP request using the specified method, URL, headers, and body with retries on failure.
///
/// # Arguments
///
/// * `method`: The HTTP method to use for the request (GET, POST, PUT, DELETE, etc.).
/// * `url`: The URL to send the request to.
/// * `headers`: An optional set of headers to include with the request.
/// * `body`: An optional function that takes a `reqwest::RequestBuilder` and returns a new `RequestBuilder` with the request body added (JSON, form data, etc.).
///
/// This function will attempt to send the request up to MAX_RETRIES times. If a request attempt fails, it logs the error and retries the request after a delay. The delay increases with each attempt.
///
/// # Returns
///
/// * `Result<Response>`: The HTTP response, or an error if all attempts to send the request fail.
///   If all attempts fail, the error message includes retry count and the last underlying request error.
///
/// # Errors
///
/// This function will return an `Err` if the request fails to send after MAX_RETRIES attempts.
///
/// # Example
///
/// ```
/// let method = Method::GET;
/// let url = "http://tw.yahoo.com";
/// let headers = Some(HeaderMap::new());
/// let body = Some(|rb: RequestBuilder| rb.json(&data));
///
/// let response = send(method, url, headers, body).await?;
/// ```
async fn send(
    method: Method,
    url: &str,
    headers: Option<header::HeaderMap>,
    body: Option<impl FnOnce(RequestBuilder) -> RequestBuilder>,
) -> Result<Response> {
    let visit_log = format!("{method}:{url}");
    let client = get_client()?;
    let mut rb = client.request(method, url);
    let mut last_error = String::new();

    if let Some(h) = headers {
        rb = rb.headers(h);
    }

    if let Some(body_fn) = body {
        rb = body_fn(rb);
    }

    for attempt in 1..=MAX_RETRIES {
        let msg = format!("Attempt {} to send {}", attempt, visit_log);
        let rb_clone = rb
            .try_clone()
            .ok_or_else(|| anyhow!("Failed to clone RequestBuilder"))?;
        let permit = SEMAPHORE.acquire().await;
        let start = Instant::now();
        let res = rb_clone.send().await;
        let elapsed = start.elapsed().as_millis();

        // 請求延遲，避免被目標網站封禁
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(permit);

        match res {
            Ok(response) => {
                LOGGER.info(format!("{} {} ms", msg, elapsed));
                //let text = response.text().await?; // Here we take ownership of response
                //LOGGER.info(format!("Response text: {}", text));
                return Ok(response);
            }
            Err(why) => {
                last_error = format!("{:?}", why);
                LOGGER.error(format!("{} failed because {:?}. {} ms", msg, why, elapsed));
                if attempt < MAX_RETRIES {
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt as u32))).await;

                    continue;
                }
            }
        }
    }

    Err(anyhow!(
        "Failed to send request to {} after {} attempts; last error: {}",
        url,
        MAX_RETRIES,
        last_error
    ))
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use concat_string::concat_string;

    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_request() {
        let url = concat_string!(
            "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
            Local::now().format("%Y%m%d").to_string(),
            "&_=",
            Local::now().timestamp_millis().to_string()
        );

        logging::debug_file_async(format!("request_get:{:?}", get(&url, None).await));

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
                logging::error_file_async(format!("Failed to get because {:?}", why));
            }
        }
    }
}
