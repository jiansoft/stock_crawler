//! 公網 IP 查詢的測試；連外的測試標 #[ignore]。

use std::net::IpAddr;

use anyhow::anyhow;

use super::*;

/// 驗證純文字 IP 即使夾帶換行，也會先 trim 再正常回傳。
#[test]
fn test_normalize_public_ip_trims_and_accepts_ipv4() {
    let ip = normalize_public_ip("ipify", " 1.2.3.4\r\n").unwrap();

    assert_eq!(ip, "1.2.3.4");
}

/// 驗證 IPv6 內容也可通過驗證並回傳標準格式。
#[test]
fn test_normalize_public_ip_accepts_ipv6() {
    let ip = normalize_public_ip("ipinfo", "2001:db8::1").unwrap();

    assert_eq!(ip, "2001:db8::1");
}

/// 驗證空白或空字串不會被誤判成成功的 IP 回應。
#[test]
fn test_normalize_public_ip_rejects_empty_response() {
    let err = normalize_public_ip("seeip", " \n ").unwrap_err();

    assert!(err.to_string().contains("empty response"));
}

/// 驗證錯頁或其他非 IP 內容會被擋下，不會直接流進 DDNS 流程。
#[test]
fn test_normalize_public_ip_rejects_non_ip_body() {
    let err = normalize_public_ip("seeip", "<html>rate limited</html>").unwrap_err();

    assert!(err.to_string().contains("invalid ip response"));
}

/// 驗證 `get_public_ip` 的單一來源處理邏輯會接受可 trim 的合法 IPv4。
#[test]
fn test_get_public_ip_collects_trimmed_ipv4_response() {
    let mut errors = Vec::new();

    let ip = collect_public_ip_result("ipify", Ok(" 1.2.3.4\r\n".to_string()), &mut errors);

    assert_eq!(ip, Some("1.2.3.4".to_string()));
    assert!(errors.is_empty());
}

/// 驗證 `get_public_ip` 的單一來源處理邏輯會把非 IP 內容記成錯誤。
#[test]
fn test_get_public_ip_collects_invalid_body_as_error() {
    let mut errors = Vec::new();

    let ip = collect_public_ip_result(
        "ipinfo",
        Ok("<html>challenge</html>".to_string()),
        &mut errors,
    );

    assert_eq!(ip, None);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("invalid ip response"));
}

/// 驗證 `get_public_ip` 的單一來源處理邏輯會保留原始服務錯誤。
#[test]
fn test_get_public_ip_collects_upstream_error() {
    let mut errors = Vec::new();

    let ip = collect_public_ip_result("seeip", Err(anyhow!("timeout")), &mut errors);
    assert_eq!(ip, None);
    assert_eq!(errors, vec!["seeip: timeout".to_string()]);
}

/// Live 測試：直接呼叫 `get_public_ip()` 取得目前對外 IP。
///
/// 此測試會對外發 HTTP 請求，因此預設標記為 `ignored`，
/// 需要時再手動執行。
#[tokio::test]
#[ignore]
async fn test_get_public_ip() {
    let ip = get_public_ip().await.unwrap();

    println!("public_ip={ip}");
    assert!(ip.parse::<IpAddr>().is_ok());
}
