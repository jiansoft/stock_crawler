use anyhow::*;
use once_cell::{sync::Lazy, sync::OnceCell};
use reqwest::{header, Client, Method};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{result::Result::Ok, time::Duration};
use tokio::sync::Semaphore;

pub(crate) static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| {
    let cpus = num_cpus::get();
    Semaphore::new(cpus * 4)
});

pub(crate) static CLIENT: OnceCell<Client> = OnceCell::new();

#[derive(Serialize, Deserialize)]
/// for Request or Response
pub struct Empty {}

fn get_client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .no_proxy()
            .pool_idle_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create reqwest client")
    })
}

/// Perform a GET request and deserialize the JSON response
pub async fn request_get_use_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    let rb = get_client().request(Method::GET, url);
    let res = request_send(rb).await?;
    res.json::<T>().await.map_err(From::from)
}

/// Perform a GET request and return the response as text
pub async fn request_get(url: &str) -> Result<String> {
    let rb = get_client().request(Method::GET, url);
    let res = request_send(rb).await?;
    res.text().await.map_err(From::from)
}

/// Perform a GET request and return the response as Big5 encoded text
pub async fn request_get_use_big5(url: &str) -> Result<String> {
    let rb = get_client().request(Method::GET, url);
    let res = request_send(rb).await?;
    res.text_force_charset("Big5").await.map_err(From::from)
}

/// Perform a POST request with JSON request and response, with specified headers
pub async fn request_post<REQ: Serialize, RES: DeserializeOwned>(
    url: &str,
    headers: Option<header::HeaderMap>,
    req: Option<&REQ>,
) -> Result<RES> {
    let mut rb = get_client().request(Method::POST, url);

    if let Some(h) = headers {
        rb = rb.headers(h);
    }

    if let Some(r) = req {
        rb = rb.json(r);
    }

    let res = request_send(rb).await?;
    res.json::<RES>().await.map_err(From::from)
}

async fn request_send(request_builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let _permit = SEMAPHORE.acquire().await?;
    Ok(request_builder.send().await?)
}

#[cfg(test)]
mod tests {
    use crate::logging;
    use chrono::Local;
    use concat_string::concat_string;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_request() {
        let url = concat_string!(
            "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
            Local::now().format("%Y%m%d").to_string(),
            "&_=",
            Local::now().timestamp_millis().to_string()
        );

        logging::info_file_async(format!("visit url:{}", url,));
        logging::info_file_async(format!("request_get:{:?}", request_get(&url).await));

        let bytes = reqwest::get("http://httpbin.org/ip")
            .await
            .unwrap()
            .bytes()
            .await;

        println!("bytes: {:#?}", bytes);
    }
}
