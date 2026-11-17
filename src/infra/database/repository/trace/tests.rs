//! 股價追蹤倉儲的測試。

use super::*;
use rust_decimal_macros::dec;

/// 本人只讀本人、家庭只讀同家未刪除會員；偽造家庭／會員配對無法取得擁有者名稱。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離的 PostgreSQL")]
async fn trace_notification_owners_enforce_household_and_member_boundaries() {
    dotenvy::dotenv().ok();
    let mut tx = database::get_tx().await.expect("開啟測試交易");
    let household: i64 = sqlx::query_scalar(
        "INSERT INTO member_household (name) VALUES ('__trace_owner_test__') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("建立測試家庭");
    let mut ids = Vec::new();
    for (name, deleted) in [("甲_[家人]", false), ("乙", false), ("已刪除", true)] {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "Member" ("Account", "Name", household_id, deleted_at)
               VALUES ($1, $1, $2, CASE WHEN $3 THEN now() END) RETURNING "Id""#,
        )
        .bind(name)
        .bind(household)
        .bind(deleted)
        .fetch_one(&mut *tx)
        .await
        .expect("建立測試會員");
        ids.push(id);
    }
    let family = fetch_visible_owners_from(&mut *tx, HoldingView::household(household))
        .await
        .expect("家庭範圍");
    assert_eq!(
        family,
        [(ids[0], "甲_[家人]".into()), (ids[1], "乙".into())]
    );
    let own = fetch_visible_owners_from(&mut *tx, HoldingView::member(household, ids[0]))
        .await
        .expect("本人範圍");
    assert_eq!(own, [(ids[0], "甲_[家人]".into())]);
    let foreign = fetch_visible_owners_from(&mut *tx, HoldingView::member(1, ids[0]))
        .await
        .expect("不同家庭");
    assert!(foreign.is_empty());
    tx.rollback().await.expect("回滾測試資料");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_trace_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgTraceRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgTraceRepository::new();
    let test_symbol = "__TEST_TRACE__";
    let test_account = "__test_trace__";

    // 1. 清理潛在的殘留測試數據（刪除測試會員時，trace 依外來鍵一併刪除）
    sqlx::query("DELETE FROM \"trace\" WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await
        .ok();
    sqlx::query("DELETE FROM \"Member\" WHERE \"Account\" = $1")
        .bind(test_account)
        .execute(database::get_connection())
        .await
        .ok();

    // 2. trace.member_id 有指向 "Member" 的外來鍵，先建立測試會員再寫入
    let member_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"Member\" (\"Account\", \"Name\", household_id) VALUES ($1, $1, 1) RETURNING \"Id\"",
    )
    .bind(test_account)
    .fetch_one(database::get_connection())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO \"trace\" (member_id, stock_symbol, floor, ceiling) VALUES ($1, $2, $3, $4)",
    )
    .bind(member_id)
    .bind(test_symbol)
    .bind(dec!(100.5))
    .bind(dec!(200.5))
    .execute(database::get_connection())
    .await
    .unwrap();

    // 3. 呼叫倉儲的 fetch_all 驗證是否能載入該測試資料
    let traces = repo.fetch_all().await.unwrap();
    let found = traces.iter().find(|t| t.stock_symbol == test_symbol);
    assert!(found.is_some());
    let found_trace = found.unwrap();
    assert_eq!(found_trace.member_id, member_id);
    assert_eq!(found_trace.floor, dec!(100.5));
    assert_eq!(found_trace.ceiling, dec!(200.5));

    // 4. 清理測試數據
    sqlx::query("DELETE FROM \"trace\" WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await
        .unwrap();
    sqlx::query("DELETE FROM \"Member\" WHERE \"Id\" = $1")
        .bind(member_id)
        .execute(database::get_connection())
        .await
        .unwrap();
}

/// 過渡唯一索引 `idx_trace_legacy_symbol`：舊版 stock_go 的 `/trace set`
/// （`ON CONFLICT (stock_symbol)`，不帶 member_id、預設歸 #1）在新主鍵下仍可執行。
/// 上線程序 L2 移除過渡索引時一併刪除這個測試。全部在交易內完成並回滾。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn legacy_trace_upsert_by_symbol_still_works() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 legacy_trace_upsert_by_symbol_still_works：無資料庫連接");
        return;
    }
    let mut tx = database::get_tx().await.expect("開啟交易");
    // 舊寫法不帶 member_id，預設歸會員 #1；測試庫可能沒有 #1，在交易內補一筆
    sqlx::query(
        r#"INSERT INTO "Member" ("Id", "Account", "Name", household_id)
           VALUES (1, '__legacy_trace__', '__legacy_trace__', 1) ON CONFLICT ("Id") DO NOTHING"#,
    )
    .execute(&mut *tx)
    .await
    .expect("確保會員 #1 存在");
    for floor in [dec!(10), dec!(11)] {
        sqlx::query(
            r#"INSERT INTO "trace" (stock_symbol, floor, ceiling) VALUES ('__TEST_LEGACY__', $1, 20)
               ON CONFLICT (stock_symbol) DO UPDATE SET floor = EXCLUDED.floor"#,
        )
        .bind(floor)
        .execute(&mut *tx)
        .await
        .expect("舊寫法寫入追蹤條件");
    }
    let rows: Vec<(i64, rust_decimal::Decimal)> = sqlx::query_as(
        r#"SELECT member_id, floor FROM "trace" WHERE stock_symbol = '__TEST_LEGACY__'"#,
    )
    .fetch_all(&mut *tx)
    .await
    .expect("讀回");
    tx.rollback().await.expect("回滾測試資料");
    assert_eq!(rows, [(1, dec!(11))]);
}
