//! 會員每日市值表的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_upsert：無資料庫連接");
        return;
    }
    tracing::debug!("開始 DailyMoneyHistoryMember::upsert");
    let current_date = NaiveDate::parse_from_str("2023-08-30", "%Y-%m-%d").unwrap();
    let mut tx = database::get_tx().await.ok();

    match DailyMoneyHistoryMember::upsert(current_date, &mut tx).await {
        Ok(r) => {
            tracing::debug!("DailyMoneyHistoryMember::upsert:{:#?}", r);
            if let Some(tx) = tx {
                tx.commit().await.expect("commit is failed");
            }
        }
        Err(why) => {
            tracing::debug!(
                "Failed to DailyMoneyHistoryMember::upsert because {:?}",
                why
            );
            if let Some(tx) = tx {
                tx.rollback().await.expect("rollback is failed");
            }
        }
    }

    tracing::debug!("結束 DailyMoneyHistoryMember::upsert");
}

/// 結算對象：未刪除的會員（含已停用）即使沒有持股也有每日市值列；已刪除的不計入；
/// 建立日晚於結算日的會員不會出現在較早的日期。全部在交易內完成並回滾。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert_covers_all_undeleted_members() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_upsert_covers_all_undeleted_members：無資料庫連接");
        return;
    }
    let date = NaiveDate::from_ymd_opt(1987, 7, 20).expect("固定日期合法");
    let mut tx = Some(database::get_tx().await.expect("開啟交易"));
    let mut ids = Vec::new();
    {
        let conn = tx.as_mut().expect("交易已開啟");
        // (帳號, 啟用, 已刪除, 建立時間)
        for (account, active, deleted, created) in [
            ("__dmh_active__", true, false, "1987-07-01 12:00+08"),
            ("__dmh_inactive__", false, false, "1987-07-01 12:00+08"),
            ("__dmh_deleted__", true, true, "1987-07-01 12:00+08"),
            ("__dmh_future__", true, false, "1987-07-21 12:00+08"),
        ] {
            let id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "Member" ("Account", "Name", household_id, is_active, deleted_at, "CreateTime")
                   VALUES ($1, $1, 1, $2, CASE WHEN $3 THEN now() END, $4::timestamptz)
                   RETURNING "Id""#,
            )
            .bind(account)
            .bind(active)
            .bind(deleted)
            .bind(created)
            .fetch_one(&mut **conn)
            .await
            .expect("建立測試會員");
            ids.push(id);
        }
        // 已刪除的會員即使還有持股列也不計入
        sqlx::query(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, date)
             VALUES ($1, '79979SA', 1, '1987-07-01')",
        )
        .bind(ids[2])
        .execute(&mut **conn)
        .await
        .expect("種入已刪除會員的持股");
    }

    DailyMoneyHistoryMember::upsert(date, &mut tx)
        .await
        .expect("會員市值結算");
    let conn = tx.as_mut().expect("交易已開啟");
    let rows: Vec<(i64, Decimal)> = sqlx::query_as(
        "SELECT member_id, market_value FROM daily_money_history_member
         WHERE date = $1 AND member_id = ANY($2) ORDER BY member_id",
    )
    .bind(date)
    .bind(&ids)
    .fetch_all(&mut **conn)
    .await
    .expect("讀取會員市值");
    tx.take()
        .expect("交易已開啟")
        .rollback()
        .await
        .expect("回滾測試資料");

    assert_eq!(rows, vec![(ids[0], Decimal::ZERO), (ids[1], Decimal::ZERO)]);
}
