//! mod的測試；連外的測試標 #[ignore]。

use crate::infra::crawler::log_public_ip_visit_test;

use super::*;

/// 驗證 serde 欄位對應：`ipString` / `ipType` 需要 rename，多餘欄位要被忽略。
#[test]
fn api_response_deserializes_official_shape() {
    let body = r#"{
        "ipString": "203.0.113.7",
        "ipType": "v4",
        "ipNumeric": "3405803783"
    }"#;
    let res: ApiResponse = serde_json::from_str(body).unwrap();

    assert_eq!(res.ip_string, "203.0.113.7");
    assert_eq!(res.ip_type, "v4");
    assert_eq!(extract_ip(res).unwrap(), "203.0.113.7");
}

/// `ipString` 為空字串時不可當成有效 IP 回傳。
#[test]
fn extract_ip_rejects_empty_ip_string() {
    let res = ApiResponse {
        ip_string: String::new(),
        ip_type: "v4".to_string(),
    };
    let err = extract_ip(res).expect_err("empty ipString should be an error");

    assert!(err.to_string().contains("can't get public ip"));
    assert!(err.to_string().contains(HOST));
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    log_public_ip_visit_test(visit).await;
}
