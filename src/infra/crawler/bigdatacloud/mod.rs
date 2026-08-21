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
mod tests {
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
}
