use anyhow::*;
use once_cell::sync::Lazy;
use reqwest::{Client, IntoUrl};
use serde::de::DeserializeOwned;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

use std::result::Result::Ok;

static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| {
    let cpus = num_cpus::get();
    Semaphore::new(cpus)
});

static CLIENT: Lazy<Arc<Mutex<Client>>> = Lazy::new(|| {
    Arc::new(Mutex::new(
        Client::builder()
            .pool_idle_timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
    ))
});

/// 封裝 reqwest 的操作
pub async fn do_request_get<T: DeserializeOwned>(url: &str) -> Result<T> {
    let _permit = SEMAPHORE.acquire().await?;
    let res: reqwest::Response;
    {
        res = CLIENT.lock().await.get(url).send().await?;
    }

    Ok(res.json::<T>().await?)
}

pub async fn request_get<T: IntoUrl>(url: T) -> Result<String> {
    let _permit = SEMAPHORE.acquire().await?;
    let res: reqwest::Response;
    {
        res = CLIENT.lock().await.get(url).send().await?;
    }

    Ok(res.text().await?)
}

pub async fn request_get_use_big5<T: IntoUrl>(url: T) -> Result<String> {
    let _permit = SEMAPHORE.acquire().await?;
    let res: reqwest::Response;
    {
        res = CLIENT.lock().await.get(url).send().await?;
    }

    Ok(res.text_force_charset("Big5").await?)
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
        logging::info_file_async(format!("request_get:{:?}", request_get(url).await));

        let bytes = reqwest::get("http://httpbin.org/ip")
            .await
            .unwrap()
            .bytes()
            .await;

        println!("bytes: {:#?}", bytes);
    }
}
