use std::sync::OnceLock;

use anyhow::Result;

use crate::core::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "ipinfo.io";

/// 組出 ipinfo 純文字 IP 端點的網址。
///
/// 抽成純函式讓網址格式可被單元測試鎖定（`/ip` 回傳純文字，不是 JSON）。
fn build_url() -> String {
    format!("https://{host}/ip", host = HOST,)
}

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(build_url);
    util::http::get(url, None).await
}

#[cfg(test)]
mod tests;
