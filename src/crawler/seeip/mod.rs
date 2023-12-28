use std::sync::OnceLock;

use anyhow::Result;

use crate::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "ipv4.seeip.org";

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(|| format!("https://{host}", host = HOST,));
    util::http::get(url, None).await
}