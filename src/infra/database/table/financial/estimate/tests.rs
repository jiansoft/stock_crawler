use crate::infra::cache::SHARE;
use chrono::Datelike;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 Estimate::upsert");

    let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
    let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
    let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
    let years_str = years_vec.join(",");
    let estimate = Estimate::new("9921".to_string(), current_date);

    match estimate.upsert(years_str).await {
        Ok(r) => tracing::debug!("Estimate::upsert:{:#?}", r),
        Err(why) => {
            tracing::debug!("Failed to Estimate::upsert because {:?}", why);
        }
    }

    tracing::debug!("結束 Estimate::upsert");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert_all() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 Estimate::upsert_all");

    let current_date = NaiveDate::parse_from_str("2026-03-03", "%Y-%m-%d").unwrap();
    let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
    let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
    let years_str = years_vec.join(",");
    // 分桶結果必須與不分桶一致：兩者都是 upsert，影響筆數即輸出的股票數。
    let unbucketed = run_estimate_upsert(current_date, &years_str, None, (1, 0))
        .await
        .expect("不分桶重建應成功");
    let bucketed = Estimate::upsert_all(current_date, years_str)
        .await
        .expect("分桶重建應成功");
    assert_eq!(bucketed.rows_affected(), unbucketed.rows_affected());

    tracing::debug!("結束 Estimate::upsert_all");
}
