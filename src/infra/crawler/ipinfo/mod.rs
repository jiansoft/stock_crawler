use std::sync::OnceLock;

use anyhow::Result;

use crate::core::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "ipinfo.io";

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(|| format!("https://{host}/ip", host = HOST,));
    util::http::get(url, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        match visit().await {
            Ok(ip) => {
                print!("{}", ip)
            }
            Err(why) => {
                tracing::error!("Failed to get because {:?}", why);
            }
        }
    }
}
