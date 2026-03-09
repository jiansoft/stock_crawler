use std::sync::OnceLock;

use anyhow::Result;

use crate::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "ipconfig.io";

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    let url = DDNS_URL.get_or_init(|| format!("https://{host}/ip", host = HOST,));
    let ip = util::http::get(url, None).await?;
    Ok(ip.trim().to_string())
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
