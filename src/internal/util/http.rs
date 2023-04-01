use anyhow::*;
use once_cell::sync::Lazy;
use reqwest::{Client, IntoUrl};
use serde::de::DeserializeOwned;
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

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
    let res: reqwest::Response;
    {
        res = CLIENT.lock().await.get(url).send().await?;
    }

    Ok(res.json::<T>().await?)
}

pub async fn request_get<T: IntoUrl>(url: T) -> Result<String> {
    let res: reqwest::Response;
    {
        res = CLIENT.lock().await.get(url).send().await?;
    }

    Ok(res.text().await?)
}
