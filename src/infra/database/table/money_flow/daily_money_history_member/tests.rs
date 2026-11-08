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
