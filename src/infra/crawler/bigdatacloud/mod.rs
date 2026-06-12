use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_derive::Serialize;

use crate::core::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "api.bigdatacloud.net";

#[derive(Serialize, Deserialize)]
struct ApiResponse {
    #[serde(rename = "ipString")]
    pub ip_string: String,
    #[serde(rename = "ipType")]
    pub ip_type: String,
}

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(|| format!("https://{host}/data/client-ip", host = HOST,));
    let res = util::http::get_json::<ApiResponse>(url).await?;
    if !res.ip_string.is_empty() {
        return Ok(res.ip_string);
    }

    Err(anyhow!("can't get public ip from {}", HOST))
}

#[cfg(test)]
mod tests {
    use crate::infra::crawler::log_public_ip_visit_test;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        log_public_ip_visit_test(visit).await;
    }
}
