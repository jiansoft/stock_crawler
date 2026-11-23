//! 持股可見範圍解析的測試。

use super::*;

/// 指定成員時不查資料庫。
#[tokio::test]
async fn single_member_does_not_query_database() {
    let ids = member_ids(&HoldingView::member(1, 7))
        .await
        .expect("不查資料庫");
    assert_eq!(ids, vec![7]);
}

/// 全體成員為該家庭未刪除的成員（含已停用），其他家庭的成員不在內。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn household_resolves_to_its_members_only() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 household_resolves_to_its_members_only：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let expected: Vec<i64> = sqlx::query_scalar(
        r#"SELECT "Id" FROM "Member" WHERE household_id = 1 AND deleted_at IS NULL ORDER BY "Id""#,
    )
    .fetch_all(pool)
    .await
    .expect("讀取成員");
    let ids = member_ids(&HoldingView::default_household())
        .await
        .expect("解析家庭");
    assert_eq!(ids, expected);
    let missing = member_ids(&HoldingView::household(987_654))
        .await
        .expect("不存在的家庭");
    assert!(missing.is_empty(), "不存在的家庭沒有任何成員");
}
