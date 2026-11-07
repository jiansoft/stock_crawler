//! money_flow的測試；連外的測試標 #[ignore]。

use super::*;
use crate::infra::database;

#[tokio::test]
#[ignore]
async fn test_money_flow_repository_flow() {
    // 載入環境變數
    dotenvy::dotenv().ok();

    if database::ping().await.is_err() {
        println!("跳過 test_money_flow_repository_flow：無資料庫連接");
        return;
    }

    // 建立資金流向與市值倉儲實例
    let repo = PgMoneyFlowRepository::new();
    // 選擇有完整資料之特定測試交易日
    let test_date = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();

    // 1. 執行 recalculate_and_save_money_flow，驗證多表交易式批次更新是否可順利執行
    let result = repo.recalculate_and_save_money_flow(test_date).await;
    assert!(
        result.is_ok(),
        "recalculate_and_save_money_flow failed: {:?}",
        result.err()
    );

    // 2. 驗證讀取會員對照資料是否正常
    let compare_data = repo
        .fetch_member_money_history_with_previous_day(test_date)
        .await;
    assert!(
        compare_data.is_ok(),
        "fetch_member_money_history_with_previous_day failed: {:?}",
        compare_data.err()
    );
    let compare_data = compare_data.unwrap();
    // 預期至少有合計列 (member_id = 0)
    assert!(!compare_data.is_empty());
    assert!(compare_data.iter().any(|r| r.member_id == 0));
}

/// 固定歷史日期：重算流程整段 transaction 可執行，會員市值對照取最近兩個有資料的日期。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn money_flow_repository_pairs_member_values_with_previous_day() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 money_flow_repository_pairs_member_values_with_previous_day：無資料庫連接");
        return;
    }
    const MEMBER: i64 = 979_790;
    let previous = NaiveDate::from_ymd_opt(1990, 3, 1).expect("固定日期合法");
    let current = NaiveDate::from_ymd_opt(1990, 3, 2).expect("固定日期合法");
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_money_history_detail_more WHERE date = ANY($1)",
            "DELETE FROM daily_money_history_detail WHERE date = ANY($1)",
            "DELETE FROM daily_money_history_member WHERE date = ANY($1)",
            "DELETE FROM daily_money_history WHERE date = ANY($1)",
            "DELETE FROM daily_stock_price_stats WHERE date = ANY($1)",
        ] {
            let _ = sqlx::query(sql)
                .bind([previous, current])
                .execute(pool)
                .await;
        }
    };
    cleanup().await;

    let repo = PgMoneyFlowRepository::new();
    repo.recalculate_and_save_money_flow(current)
        .await
        .expect("重算流程應成功");

    // 以種入的會員市值取代重算結果，驗證對照：前一日 100、當日 120。
    cleanup().await;
    sqlx::query(
        "INSERT INTO daily_money_history_member (date, member_id, market_value)
         VALUES ($1, $3, 100), ($2, $3, 120)",
    )
    .bind(previous)
    .bind(current)
    .bind(MEMBER)
    .execute(pool)
    .await
    .expect("種入會員市值");
    let rows = repo
        .fetch_member_money_history_with_previous_day(current)
        .await
        .expect("讀取會員對照");
    let row = rows
        .iter()
        .find(|row| row.member_id == MEMBER)
        .expect("應有測試會員");
    assert_eq!((row.date, row.previous_date), (current, Some(previous)));
    assert_eq!(
        (row.market_value, row.previous_market_value),
        (
            rust_decimal::Decimal::from(120),
            rust_decimal::Decimal::from(100)
        )
    );

    cleanup().await;
}
