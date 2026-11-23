//! notify_topic 寫入的整合測試（需要隔離的測試資料庫）。

use std::collections::HashSet;

use super::*;
use crate::infra::database;

/// 每個 Topic 都存在於 notify_topic，且欄位與目錄一致；重複執行不出錯（5-01 驗收）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn every_topic_exists_in_notify_topic() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 every_topic_exists_in_notify_topic：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let written = upsert_catalog(pool).await.expect("第一次寫入");
    assert_eq!(written, Topic::ALL.len() as u64, "每個項目寫入一列");
    upsert_catalog(pool).await.expect("重複寫入不應失敗");

    let rows: Vec<(String, String, String, bool, bool)> = sqlx::query_as(
        "SELECT code, audience, required_permission, root_only, default_enabled FROM notify_topic",
    )
    .fetch_all(pool)
    .await
    .expect("讀取 notify_topic");
    let stored: HashSet<&str> = rows.iter().map(|row| row.0.as_str()).collect();
    for topic in Topic::ALL {
        assert!(
            stored.contains(topic.code()),
            "notify_topic 缺少 {}",
            topic.code()
        );
        let spec = topic.spec();
        let row = rows
            .iter()
            .find(|row| row.0 == spec.code)
            .expect("已確認存在");
        assert_eq!(
            (row.1.as_str(), row.2.as_str(), row.3, row.4),
            (
                spec.audience.as_str(),
                spec.required_permission,
                spec.root_only,
                spec.default_enabled
            ),
            "{} 的欄位應與目錄一致",
            spec.code
        );
    }
}
