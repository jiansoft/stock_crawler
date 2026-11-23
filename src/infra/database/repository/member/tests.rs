//! 真實會員名稱查詢：新增、改名、特殊字元與家庭邊界。

use super::*;

/// 六位會員一次載入，停用會員仍可列歷史持股；已刪除與其他家庭均排除。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離的 PostgreSQL")]
async fn notification_names_follow_new_members_and_renames() {
    dotenvy::dotenv().ok();
    let mut tx = database::get_tx().await.expect("開啟測試交易");
    let household: i64 = sqlx::query_scalar(
        "INSERT INTO member_household (name) VALUES ('__notification_names__') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("建立測試家庭");
    let mut ids = Vec::new();
    for (index, name) in ["原名", "二", "三", "四", "第五_[甲]", "第六`乙`"]
        .into_iter()
        .enumerate()
    {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "Member" ("Account", "Name", household_id, is_active)
               VALUES ($1, $2, $3, $4) RETURNING "Id""#,
        )
        .bind(format!("__names_{household}_{index}"))
        .bind(name)
        .bind(household)
        .bind(index != 5)
        .fetch_one(&mut *tx)
        .await
        .expect("新增會員");
        ids.push(id);
    }
    let view = HoldingView::household(household);
    let names = fetch_names_from(&mut *tx, &view)
        .await
        .expect("批次讀取六位會員");
    assert_eq!(names.len(), 6);
    assert_eq!(names[&ids[4]], "第五_[甲]");
    assert_eq!(names[&ids[5]], "第六`乙`");
    sqlx::query(r#"UPDATE "Member" SET "Name" = '改名_[家人]' WHERE "Id" = $1"#)
        .bind(ids[0])
        .execute(&mut *tx)
        .await
        .expect("改名");
    let renamed = fetch_names_from(&mut *tx, &view)
        .await
        .expect("下一次通知重新載入");
    let label = crate::app::event::taiwan_stock::member_label(&renamed, ids[0]);
    assert_eq!(
        crate::core::util::text::escape_markdown_v2(label),
        r"改名\_\[家人\]"
    );
    assert_eq!(renamed[&ids[0]], "改名_[家人]");
    let own = fetch_names_from(&mut *tx, &HoldingView::member(household, ids[4]))
        .await
        .expect("本人範圍");
    assert_eq!(own, BTreeMap::from([(ids[4], "第五_[甲]".to_owned())]));
    let foreign = fetch_names_from(&mut *tx, &HoldingView::member(1, ids[4]))
        .await
        .expect("跨家庭範圍");
    assert!(foreign.is_empty());
    sqlx::query(r#"UPDATE "Member" SET deleted_at = now() WHERE "Id" = $1"#)
        .bind(ids[4])
        .execute(&mut *tx)
        .await
        .expect("軟刪除");
    let after_delete = fetch_names_from(&mut *tx, &view).await.expect("刪除後範圍");
    assert!(!after_delete.contains_key(&ids[4]));
    tx.rollback().await.expect("回滾測試資料");
}
