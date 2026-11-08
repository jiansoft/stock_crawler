//! 具名策略（[`StrategyPresetRepository`]）的 PostgreSQL 實作的測試。

use super::*;

/// 新增、覆蓋（保留名稱）、讀取、列出與刪除；用固定的測試名稱並自我清理。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn strategy_preset_round_trip() {
    dotenvy::dotenv().ok();
    let repository = PgStrategyPresetRepository::new();
    let name = "79979 測試策略";
    if repository.delete(name).await.is_err() {
        println!("跳過 strategy_preset_round_trip：無資料庫連接");
        return;
    }
    let mut item = StrategyPreset {
        name: name.to_owned(),
        query: "limit=5&from=2019-01-02".to_owned(),
        description: "測試".to_owned(),
        alert: false,
    };
    repository.save(&item).await.expect("新增");
    item.alert = true;
    item.query = "limit=10&from=2019-01-02".to_owned();
    repository.save(&item).await.expect("覆蓋");
    assert_eq!(
        repository.find(name).await.expect("讀取"),
        Some(item.clone())
    );
    assert!(
        repository
            .list()
            .await
            .expect("列出")
            .iter()
            .any(|preset| preset == &item)
    );
    assert!(repository.delete(name).await.expect("刪除"));
    assert!(!repository.delete(name).await.expect("再刪一次"));
    assert_eq!(repository.find(name).await.expect("讀取"), None);
}
