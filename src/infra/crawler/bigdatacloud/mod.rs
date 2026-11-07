use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

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

/// 從 API 回應取出公開 IP。
///
/// 純函式（不做網路 I/O）：`ipString` 為空字串時代表來源沒給有效值，
/// 必須報錯，避免把空字串當成 IP 傳給後續的 DDNS 更新流程。
fn extract_ip(res: ApiResponse) -> Result<String> {
    if !res.ip_string.is_empty() {
        return Ok(res.ip_string);
    }

    Err(anyhow!("can't get public ip from {}", HOST))
}

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(|| format!("https://{host}/data/client-ip", host = HOST,));
    let res = util::http::get_json::<ApiResponse>(url).await?;

    extract_ip(res)
}

#[cfg(test)]
mod tests;
