use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_derive::Serialize;

use crate::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "api.myip.com";

#[derive(Serialize, Deserialize)]
struct MyIpResponse {
    pub ip: String,
    pub country: String,
    pub cc: String,
}

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(|| format!("https://{host}", host = HOST,));
    let res = util::http::get_json::<MyIpResponse>(url).await?;
    if !res.ip.is_empty() {
        return Ok(res.ip);
    }

    Err(anyhow!("can't get public ip from {}", HOST))
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        match visit().await {
            Ok(ip) => {
                dbg!(ip);
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to get because {:?}", why));
            }
        }
    }
}
