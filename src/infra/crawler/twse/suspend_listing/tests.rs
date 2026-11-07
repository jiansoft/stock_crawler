//! suspend_listing的測試；連外的測試標 #[ignore]。

use std::result::Result::Ok;

use crate::infra::cache::SHARE;

use super::*;

#[test]
fn test_suspend_listing_deserialize() {
    let json = r#"[
        {"DelistingDate":"2024/01/15","Company":"測試下市公司","Code":"9999"},
        {"DelistingDate":"2023/06/30","Company":"另一家公司","Code":"8888"}
    ]"#;
    let result: Vec<SuspendListing> = serde_json::from_str(json).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].stock_symbol, "9999");
    assert_eq!(result[0].name, "測試下市公司");
    assert_eq!(result[0].delisting_date, "2024/01/15");
    assert_eq!(result[1].stock_symbol, "8888");
}

#[test]
fn test_suspend_listing_deserialize_empty() {
    let result: Vec<SuspendListing> = serde_json::from_str("[]").unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 visit");

    match visit().await {
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
        Ok(list) => {
            tracing::debug!("data:{:#?}", list);
        }
    }

    tracing::debug!("結束 visit");
}
