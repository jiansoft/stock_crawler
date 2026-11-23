//! 通知路由讀取的整合測試（需要隔離的測試資料庫）。

use super::*;
use crate::{
    core::notify_topic::Topic, domain::portfolio::view::HoldingView,
    infra::database::repository::notify_topic::upsert_catalog,
};

/// 只回訂閱為開、管道啟用、會員可收通知的路由；含家庭的範圍為整個家庭（7.5）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn routes_follow_subscription_channel_and_member_state() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 routes_follow_subscription_channel_and_member_state：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    upsert_catalog(pool).await.expect("寫入目錄");
    let cleanup = || async {
        let _ = sqlx::query(r#"DELETE FROM "Member" WHERE "Id" IN (79971, 79972, 79973) AND "Account" LIKE '__iam_test_%'"#)
            .execute(pool)
            .await;
    };
    cleanup().await;
    for (id, must_change) in [(79971_i64, false), (79972, false), (79973, true)] {
        sqlx::query(
            r#"INSERT INTO "Member" ("Id", "Account", "Name", household_id, must_change_password)
               VALUES ($1, '__iam_test_' || $1, 'IAM 測試', 1, $2)"#,
        )
        .bind(id)
        .bind(must_change)
        .execute(pool)
        .await
        .expect("種入測試會員");
        sqlx::query(
            "INSERT INTO member_notify_channel (member_id, kind, address, source, chat_type) VALUES ($1, 'telegram', $2, 'bind', 'private')",
        )
        .bind(id)
        .bind(format!("8{id}"))
        .execute(pool)
        .await
        .expect("種入管道");
    }
    let topic = Topic::HoldingsPayable.code();
    for (id, enabled, view) in [
        (79971_i64, true, "household"),
        (79972, false, "none"),
        (79973, true, "own"),
    ] {
        sqlx::query("INSERT INTO member_notify_subscription (member_id, topic, holdings_view, enabled) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(topic)
            .bind(view)
            .bind(enabled)
            .execute(pool)
            .await
            .expect("種入訂閱");
    }

    let routes = PgNotifyRouteRepository.fetch_routes(topic).await;
    cleanup().await;
    let routes: Vec<NotifyRoute> = routes
        .expect("讀取路由")
        .into_iter()
        .filter(|route| (79971..=79973).contains(&route.member_id))
        .collect();
    assert_eq!(routes.len(), 1, "關閉訂閱與尚未改密的會員不收：{routes:?}");
    assert_eq!(routes[0].chat_id, 879971);
    assert_eq!(routes[0].view(), HoldingView::household(1));
}
