//! 三則通知的本體（`notify_*`）：空清單不送、第一次送出、同一期重跑不重複（Redis）。
//!
//! 期別用 2099 年、代號用 79979 系列，測完刪除寫入的去重鍵。

use super::*;

/// 每個測試開頭檢查 Redis，沒有就略過。
async fn redis_available(test: &str) -> bool {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 {test}：無 Redis 連接");
        return false;
    }
    true
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn notify_revenue_sends_once_per_period() {
    if !redis_available("notify_revenue_sends_once_per_period").await {
        return;
    }
    let key = "holding_review:revenue:209901";
    let _ = CLIENT.delete(key).await;
    let alerts = [revenue_alert("79979RV", "營收測試", dec!(10), dec!(1))];

    let empty = EventDispatcher::notify_revenue(209901, &[]).await;
    let first = EventDispatcher::notify_revenue(209901, &alerts).await;
    let repeated = EventDispatcher::notify_revenue(209901, &alerts).await;
    let _ = CLIENT.delete(key).await;

    assert!(!empty, "沒有持股公布不送");
    assert!(first, "第一次送出");
    assert!(!repeated, "同一期重跑不重複");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn notify_financials_sends_once_per_quarter() {
    if !redis_available("notify_financials_sends_once_per_quarter").await {
        return;
    }
    let key = "holding_review:financial:2099Q1";
    let _ = CLIENT.delete(key).await;
    let alert = HoldingFinancialAlert {
        stock_symbol: "79979FN".to_string(),
        ..financial_alert(None)
    };
    let alerts = [alert];

    let empty = EventDispatcher::notify_financials(2099, "Q1", &[]).await;
    let first = EventDispatcher::notify_financials(2099, "Q1", &alerts).await;
    let repeated = EventDispatcher::notify_financials(2099, "Q1", &alerts).await;
    let _ = CLIENT.delete(key).await;

    assert!(!empty, "沒有持股財報不送");
    assert!(first, "第一次送出");
    assert!(!repeated, "同一季重跑不重複");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn notify_foreign_holdings_dedups_each_direction_and_symbol() {
    if !redis_available("notify_foreign_holdings_dedups_each_direction_and_symbol").await {
        return;
    }
    let keys = [
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Increase, "79979FH1"),
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Decrease, "79979FH2"),
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Increase, "79979FH3"),
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Decrease, "79979FH3"),
    ];
    for key in &keys {
        let _ = CLIENT.delete(key).await;
    }
    let date = NaiveDate::from_ymd_opt(2099, 1, 5).expect("日期應合法");
    let alerts = [
        foreign_alert("79979FH1", "增持", Some(dec!(5)), 4),
        foreign_alert("79979FH2", "減持", Some(dec!(-5)), -4),
        foreign_alert("79979FH3", "未達門檻", Some(dec!(0.1)), 1),
    ];

    let first = EventDispatcher::notify_foreign_holdings(date, &alerts).await;
    let repeated = EventDispatcher::notify_foreign_holdings(date, &alerts).await;
    let mut flags = Vec::new();
    for key in &keys {
        flags.push(EventDispatcher::is_already_notified(key).await);
        let _ = CLIENT.delete(key).await;
    }

    assert_eq!(first, 2, "增持、減持各一檔，未達門檻的不通知");
    assert_eq!(repeated, 0, "去重期間內不再通知");
    assert_eq!(flags, [true, true, false, false]);
}

/// 三個事件 handler 經由資料庫查詢後交給 `notify_*`；2099 年的期別沒有任何持股資料，不送出（臨時庫）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn handlers_query_then_notify_nothing_for_future_periods() {
    if !redis_available("handlers_query_then_notify_nothing_for_future_periods").await {
        return;
    }
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 handlers_query_then_notify_nothing_for_future_periods：無資料庫連接");
        return;
    }
    EventDispatcher::handle_monthly_revenue_updated(209901)
        .await
        .expect("月營收查詢");
    EventDispatcher::handle_quarterly_financials_updated(2099, "Q1")
        .await
        .expect("季報查詢");
    EventDispatcher::handle_foreign_holdings_updated(
        NaiveDate::from_ymd_opt(2099, 1, 5).expect("日期應合法"),
    )
    .await
    .expect("外資持股查詢");
}
