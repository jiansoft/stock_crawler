//! 每日收盤事件的測試：匯總流程、各市場成交家數短缺判斷、技術訊號／未來報酬缺漏的補算與報告。

use super::*;
use crate::infra::cache::SHARE;
use std::time::Duration;

mod derived_check;
mod mock_sources;
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

/// 週六、週日直接視為休市，不查交易所休市日清單（不連外）。
#[tokio::test]
async fn weekend_is_market_closed_without_holiday_lookup() {
    use chrono::TimeZone;

    for day in [7, 8] {
        let weekend = Local
            .with_ymd_and_hms(2026, 11, day, 15, 0, 0)
            .single()
            .expect("有效時間");
        assert!(is_market_closed(weekend).await, "2026-11-{day:02} 是週末");
    }
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
