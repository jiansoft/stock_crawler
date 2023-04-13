pub mod parse;

use anyhow::*;
use once_cell::{sync::Lazy, sync::OnceCell};
use reqwest::{header, Client, Method, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Semaphore;

pub(crate) static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| {
    let cpus = num_cpus::get();
    Semaphore::new(cpus * 4)
});

pub(crate) static CLIENT: OnceCell<Client> = OnceCell::new();

#[derive(Serialize, Deserialize)]
/// for Request or Response
pub struct Empty {}

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

/// Perform a GET request and deserialize the JSON response
pub async fn request_get_use_json<RES: DeserializeOwned>(url: &str) -> Result<RES> {
    let res = request_get_common(url).await?;
    response_with_json(res).await
}

/// Perform a GET request and return the response as text
pub async fn request_get(url: &str) -> Result<String> {
    request_get_common(url)
        .await?
        .text()
        .await
        .map_err(|e| anyhow!("Error parsing response text: {:?}", e))
}

/// Perform a GET request and return the response as Big5 encoded text
pub async fn request_get_use_big5(url: &str) -> Result<String> {
    request_get_common(url)
        .await?
        .text_force_charset("Big5")
        .await
        .map_err(|e| anyhow!("Error parsing response text use BIG5: {:?}", e))
}

/// Perform a POST request with JSON request and response, with specified headers
pub async fn request_post_use_json<REQ, RES>(
    url: &str,
    headers: Option<header::HeaderMap>,
    req: Option<&REQ>,
) -> Result<RES>
where
    REQ: Serialize,
    RES: DeserializeOwned,
{
    let client = get_client()?;
    let mut rb = client.request(Method::POST, url);

    if let Some(h) = headers {
        rb = rb.headers(h);
    }

    if let Some(r) = req {
        rb = rb.json(r);
    }

    let res = request_send(rb).await?;
    response_with_json(res).await
}

/// Perform a POST request with form data and a specified set of headers, and receive a response.
pub async fn request_post(
    url: &str,
    headers: Option<header::HeaderMap>,
    params: Option<HashMap<&str, &str>>,
) -> Result<String> {
    let client = get_client()?;
    let mut rb = client.request(Method::POST, url);

    if let Some(h) = headers {
        rb = rb.headers(h);
    }

    if let Some(p) = params {
        rb = rb.form(&p);
    }

    request_send(rb)
        .await?
        .text()
        .await
        .map_err(|e| anyhow!("Error parsing response text: {:?}", e))
}
/// 發送HTTP請求
async fn request_send(request_builder: reqwest::RequestBuilder) -> Result<Response> {
    let _permit = SEMAPHORE.acquire().await?;
    request_builder
        .send()
        .await
        .map_err(|e| anyhow!("Error sending request: {:?}", e))
}

/// 回應的數據使用json反序列成指定的 RES 類型物件
async fn response_with_json<RES: DeserializeOwned>(res: Response) -> Result<RES> {
    res.json::<RES>()
        .await
        .map_err(|e| anyhow!("Error parsing response JSON: {:?}", e))
}

async fn request_get_common(url: &str) -> Result<Response> {
    let client = get_client()?;
    let rb = client.request(Method::GET, url);
    Ok(request_send(rb).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;
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
