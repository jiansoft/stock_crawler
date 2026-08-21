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
mod tests {
    use super::*;

    /// 端點打錯會拿到 HTML 而非純文字 IP，這裡把網址格式鎖定住。
    #[test]
    fn build_url_points_to_plain_text_ip_endpoint() {
        assert_eq!(build_url(), "https://ipinfo.io/ip");
    }

    /// `DDNS_URL` 只初始化一次，重複呼叫 `visit` 必須拿到同一個網址。
    #[test]
    fn ddns_url_is_initialized_once() {
        let first = DDNS_URL.get_or_init(build_url).clone();
        let second = DDNS_URL.get_or_init(|| "https://should-not-be-used.test".to_string());

        assert_eq!(first, "https://ipinfo.io/ip");
        assert_eq!(&first, second);
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
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
