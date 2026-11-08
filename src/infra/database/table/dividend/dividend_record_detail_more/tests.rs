//! 持股逐筆股利明細表的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 test_upsert");

    let mut e = DividendRecordDetailMore::new(
        1,
        2,
        3,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
    );
    let mut tx_option: Option<Transaction<Postgres>> = database::get_tx().await.ok();
    match e.upsert(&mut tx_option).await {
        Ok(word_id) => {
            tracing::debug!("serial:{} e:{:#?}", word_id, &e);

            if let Some(mut tx) = tx_option.take() {
                let _ = sqlx::query("delete from dividend_record_detail_more where serial = $1;")
                    .bind(word_id)
                    .execute(&mut *tx)
                    .await;
            }
        }
        Err(why) => {
            tracing::debug!("Failed to upsert because:{:?}", why);
            if let Some(tx) = tx_option.take() {
                tx.rollback().await.unwrap();
            }
        }
    }

    if let Some(tx) = tx_option {
        tx.commit().await.unwrap();
    }

    tracing::debug!("結束 test_upsert");
}
