//! mod的測試；連外的測試標 #[ignore]。

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
