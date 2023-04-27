pub mod parse;

use anyhow::*;
use once_cell::{sync::Lazy, sync::OnceCell};
use reqwest::{header, Client, Method, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tokio::sync::Semaphore;

/// A semaphore for limiting concurrent requests.
///
/// The initial number of permits is set to four times the number of available CPU cores.
static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| {
    let cpus = num_cpus::get();
    Semaphore::new(cpus * 4)
});

/// A singleton instance of the reqwest client.
static CLIENT: OnceCell<Client> = OnceCell::new();

#[derive(Serialize, Deserialize)]
/// An empty struct to represent an empty request or response.
pub struct Empty {}

/// Returns the reqwest client singleton instance or creates one if it doesn't exist.
///
/// # Returns
///
/// * Result<&'static Client>: A reference to the reqwest client instance or an error if the client
/// cannot be created.
fn get_client() -> Result<&'static Client> {
    CLIENT.get_or_try_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .no_proxy()
            .pool_idle_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
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
pub async fn request_get_use_json<RES: DeserializeOwned>(url: &str) -> Result<RES> {
    let res = request_get_common(url).await?;
    response_with_json(res).await
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
pub async fn request_get(url: &str) -> Result<String> {
    request_get_common(url)
        .await?
        .text()
        .await
        .map_err(|e| anyhow!("Error parsing response text: {:?}", e))
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

pub async fn request_get_use_big5(url: &str) -> Result<String> {
    request_get_common(url)
        .await?
        .text_force_charset("Big5")
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

pub async fn request_post_use_json<REQ, RES>(
    url: &str,
    headers: Option<header::HeaderMap>,
    req: Option<&REQ>,
) -> Result<RES>
where
    REQ: Serialize,
    RES: DeserializeOwned,
{
    let res = request_post_common(url, headers, |rb| {
        if let Some(r) = req {
            rb.json(r)
        } else {
            rb
        }
    })
    .await?;
    response_with_json(res).await
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
/// * `Result<String>`: The response text, or an error if the request
/// fails or the response cannot be parsed.
pub async fn request_post(
    url: &str,
    headers: Option<header::HeaderMap>,
    params: Option<HashMap<&str, &str>>,
) -> Result<String> {
    let res = request_post_common(url, headers, |rb| {
        if let Some(p) = params {
            rb.form(&p)
        } else {
            rb
        }
    })
        .await?;
    res.text()
        .await
        .map_err(|e| anyhow!("Error parsing response text: {:?}", e))
}

/// Sends an HTTP request using a given request builder.
///
/// # Arguments
///
/// * request_builder: The request builder to use for sending the request.
///
/// # Returns
///
/// * Result<Response>: The HTTP response, or an error if the request fails.
async fn request_send(request_builder: reqwest::RequestBuilder) -> Result<Response> {
    let _permit = SEMAPHORE.acquire().await;
    request_builder
        .send()
        .await
        .map_err(|e| anyhow!("Error sending request: {:?}", e))
}

/// Deserializes the JSON response into the specified type.
///
/// # Type Parameters
///
/// * RES: The type to deserialize the JSON response into. It must implement DeserializeOwned.
///
/// # Arguments
///
/// * res: The HTTP response to deserialize from JSON.
///
/// # Returns
///
/// * Result<RES>: The deserialized response, or an error if the response cannot be deserialized.
async fn response_with_json<RES: DeserializeOwned>(res: Response) -> Result<RES> {
    res.json::<RES>()
        .await
        .map_err(|e| anyhow!("Error parsing response JSON: {:?}", e))
}

/// Common functionality for sending an HTTP GET request.
///
/// # Arguments
///
/// * url: The URL to send the GET request to.
///
/// # Returns
///
/// * Result<Response>: The HTTP response, or an error if the request fails.
async fn request_get_common(url: &str) -> Result<Response> {
    let client = get_client()?;
    let rb = client.request(Method::GET, url);
    request_send(rb).await
}

/// A common function for sending HTTP POST requests with the specified headers and request body.
///
/// # Arguments
///
/// * `url`: The URL to send the POST request to.
/// * `headers`: An optional set of headers to include with the request.
/// * `body`: A function that takes a `reqwest::RequestBuilder` and modifies it with the request body (JSON, form data, etc.).
///
/// # Returns
///
/// * `Result<Response>`: The HTTP response, or an error if the request fails.
async fn request_post_common(
    url: &str,
    headers: Option<header::HeaderMap>,
    body: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> Result<Response> {
    let client = get_client()?;
    let mut rb = client.request(Method::POST, url);

    if let Some(h) = headers {
        rb = rb.headers(h);
    }

    rb = body(rb);

    request_send(rb).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;
    use chrono::Local;
    use concat_string::concat_string;

    #[tokio::test]
    async fn test_request() {
        let url = concat_string!(
            "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
            Local::now().format("%Y%m%d").to_string(),
            "&_=",
            Local::now().timestamp_millis().to_string()
        );

        logging::debug_file_async(format!("visit url:{}", url,));
        logging::debug_file_async(format!("request_get:{:?}", request_get(&url).await));

        let bytes = reqwest::get("https://httpbin.org/ip")
            .await
            .unwrap()
            .bytes()
            .await;

        println!("bytes: {:#?}", bytes);
    }
}
