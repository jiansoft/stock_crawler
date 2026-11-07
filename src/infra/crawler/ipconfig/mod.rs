use std::sync::OnceLock;

use anyhow::Result;

use crate::infra::crawler;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "ipconfig.io";

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    Ok(crawler::get_public_ip_text(&DDNS_URL, HOST, "/ip", true).await?)
}

#[cfg(test)]
mod tests;
