//! 每日收盤事件的測試：匯總流程、各市場成交家數短缺判斷、技術訊號／未來報酬缺漏的補算與報告。

use super::*;
use crate::infra::cache::SHARE;
use std::time::Duration;

mod derived_check;
mod quote_check;

/// 每日收盤事件主要匯總流程的整合測試。
#[tokio::test]
#[ignore]
async fn test_aggregate() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    tracing::debug!("開始 event::taiwan_stock::closing::aggregate");

    let current_date = NaiveDate::parse_from_str("2026-04-30", "%Y-%m-%d").unwrap();

    match aggregate(current_date).await {
        Ok(_) => {
            tracing::debug!(
                "{}",
                "event::taiwan_stock::closing::aggregate 完成".to_string(),
            );
        }
        Err(why) => {
            tracing::debug!(
                "Failed to event::taiwan_stock::closing::aggregate because {:?}",
                why
            );
        }
    }

    tracing::debug!("結束 event::taiwan_stock::closing::aggregate");
    tokio::time::sleep(Duration::from_secs(1)).await;
}

fn count(date: NaiveDate, market_id: i32, traded: i64) -> MarketTradedCount {
    MarketTradedCount {
        date,
        market_id,
        traded,
    }
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
}
