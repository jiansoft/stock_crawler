//! 持股年度股利總計表的寫入與累計查詢測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate_cumulate_dividend() {
    dotenvy::dotenv().ok();
    // 與其他 DB 測試一致：資料庫不可用時明確跳過，而不是在
    // begin().unwrap() 上 panic（舊版在無資料庫的環境會直接紅燈）。
    if database::ping().await.is_err() {
        println!("跳過 test_calculate_cumulate_dividend：無資料庫連接");
        return;
    }
    tracing::debug!("開始 calculate_cumulate_dividend");
    let drd = DividendRecordDetail::new(
        27,
        2022,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
    );
    let mut tx_option: Option<Transaction<Postgres>> =
        Some(database::get_connection().begin().await.unwrap());
    match drd.fetch_cumulate_dividend(&mut tx_option).await {
        Ok(cd) => {
            tracing::debug!("cd: {:?}", cd);
        }
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    if let Some(tx) = tx_option {
        tx.commit().await.unwrap();
    }

    tracing::debug!("結束 calculate_cumulate_dividend");
}

/// 假持股序號（`stock_ownership_details` 沒有這筆，表上也沒有外鍵）。
const FAKE_OWNERSHIP: i64 = 979_794_001;

/// 第一次寫入走連線池、回傳新序號；同一持股同一年度再寫一次走交易，
/// 依主鍵更新金額、序號不變，交易提交後才看得到新值。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn upsert_inserts_then_updates_by_ownership_and_year() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 upsert_inserts_then_updates_by_ownership_and_year：無資料庫連接");
        return;
    }
    let cleanup = || async {
        sqlx::query("DELETE FROM dividend_record_detail WHERE stock_ownership_details_serial = $1")
            .bind(FAKE_OWNERSHIP)
            .execute(pool)
            .await
            .expect("清除測試股利明細");
    };
    cleanup().await;

    let mut first = DividendRecordDetail::new(
        FAKE_OWNERSHIP,
        1999,
        Decimal::new(1500, 0),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::new(1500, 0),
    );
    let inserted = first.upsert(&mut None).await;

    let mut second = DividendRecordDetail::new(
        FAKE_OWNERSHIP,
        1999,
        Decimal::new(2000, 0),
        Decimal::new(10, 0),
        Decimal::new(100, 0),
        Decimal::new(2100, 0),
    );
    let mut tx = Some(pool.begin().await.expect("開始交易"));
    let updated = second.upsert(&mut tx).await;
    if let Some(tx) = tx {
        tx.commit().await.expect("提交交易");
    }
    let rows: Vec<(i64, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as(
        "SELECT serial, cash, stock, stock_money, total FROM dividend_record_detail \
         WHERE stock_ownership_details_serial = $1",
    )
    .bind(FAKE_OWNERSHIP)
    .fetch_all(pool)
    .await
    .expect("讀回股利明細");
    cleanup().await;

    let inserted = inserted.expect("第一次寫入");
    let updated = updated.expect("交易內更新");
    assert!(inserted > 0, "應回傳資料庫產生的序號");
    assert_eq!(updated, inserted, "同一持股同一年度只會有一列");
    assert_eq!(second.serial, inserted, "upsert 會回填序號");
    assert_eq!(
        rows,
        vec![(
            inserted,
            Decimal::new(2000, 0),
            Decimal::new(10, 0),
            Decimal::new(100, 0),
            Decimal::new(2100, 0)
        )]
    );
}
