//! 盤中價格追蹤的測試：追蹤目標、到價通知、排程（週末、休市日、休市日表讀不到）。

use std::collections::HashMap;

use rust_decimal_macros::dec;

use super::*;

fn trace(symbol: &str, floor: Decimal, ceiling: Decimal) -> PriceTrace {
    PriceTrace {
        stock_symbol: symbol.to_string(),
        floor,
        ceiling,
    }
}

/// 快取沒有報價、報價為 0 或價格仍在區間內時，都不會觸發警報。
#[tokio::test]
async fn process_cached_targets_skips_missing_zero_and_in_range_prices() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.lock().await;
    SHARE.clear_last_trading_day_quotes();
    SHARE.clear_stock_snapshots();
    let targets = vec![trace("79985", dec!(10), dec!(20))];

    // 快取沒有這檔
    process_cached_targets(
        "79985".to_string(),
        targets.clone(),
        EvaluationSource::PriceEvent,
    )
    .await;

    // 報價為 0（尚未成交）與價格在區間內
    for price in [Decimal::ZERO, dec!(15)] {
        let mut snapshots = HashMap::new();
        snapshots.insert(
            "79985".to_string(),
            RealtimeSnapshot::new("79985".to_string(), price),
        );
        SHARE.set_stock_snapshots(snapshots);
        process_cached_targets(
            "79985".to_string(),
            targets.clone(),
            EvaluationSource::Reconciliation,
        )
        .await;
    }

    SHARE.clear_stock_snapshots();
}

/// 沒有追蹤條件時，價格事件與對帳掃描都直接結束。
#[tokio::test]
async fn evaluation_entry_points_do_nothing_without_targets() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.lock().await;
    clear_trace_targets_cache();
    evaluate_price_update("79985".to_string())
        .await
        .expect("沒有追蹤條件也應成功");
    assert_eq!(reconcile_target_prices().await.expect("對帳"), 0);
}

/// 驗證即時報價快取讀取可正確命中與 miss。
#[test]
fn test_get_cached_snapshot() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.blocking_lock();
    // 確保全域 SHARE 的昨收快取與報價快取都是乾淨的，
    // 避免其他測試在 SHARE.last_trading_day_quotes 留下真實收盤價，
    // 導致 is_valid_price 判斷 dec!(998) 偏差過大而拒絕此快照。
    SHARE.clear_last_trading_day_quotes();
    SHARE.clear_stock_snapshots();

    let mut snapshots = HashMap::new();
    let mut snapshot = RealtimeSnapshot::new("2330".to_string(), dec!(998));
    snapshot.source_site = "Yahoo".to_string();
    snapshots.insert("2330".to_string(), snapshot);
    SHARE.set_stock_snapshots(snapshots);

    let snapshot = get_cached_snapshot("2330").unwrap();
    assert_eq!(snapshot.price, dec!(998));
    assert_eq!(snapshot.source_site, "Yahoo");
    assert!(get_cached_snapshot("2317").is_none());

    SHARE.clear_stock_snapshots();
}

/// 以本機 mock server 取代證交所休市日表（G15）：週末與休市日直接結束、不啟動追蹤，
/// 休市日表讀不到時回錯誤（排程會告警），而不是當成交易日照常啟動。
mod schedule {
    use chrono::TimeZone;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const TWSE: &str = "https://www.twse.com.tw";
    const HOLIDAY_PATH: &str = "/rwd/zh/holidaySchedule/holidaySchedule";

    fn at(year: i32, month: u32, day: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, 9, 30, 0)
            .single()
            .expect("本地時間沒有歧義")
    }

    /// 1991 年的休市日表：只有 1991-10-10 國慶日。
    async fn holiday_server(year: &str, body: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(HOLIDAY_PATH))
            .and(query_param("date", year))
            .respond_with(body)
            .mount(&server)
            .await;
        server
    }

    fn holidays_1991() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "stat": "ok", "date": "19910101", "queryYear": 1991, "total": 2,
            "data": [
                ["1991-10-10", "國慶日", "依規定放假1日。"],
                ["1991-01-04", "國曆新年開始交易日", "國曆新年開始交易。"]
            ]
        }))
    }

    /// 週末直接結束，連休市日表都不查。
    #[tokio::test]
    async fn execute_at_skips_weekend_without_any_request() {
        let server = MockServer::start().await;
        let uri = server.uri();
        let result = with_origins(&[(TWSE, uri.as_str())], execute_at(at(1991, 10, 12))).await;
        result.expect("週末直接結束");
        let requests = server
            .received_requests()
            .await
            .map_or(0, |requests| requests.len());
        assert_eq!(requests, 0, "週末不查休市日表");
    }

    /// 平日但是休市日：查完休市日表就結束，不設執行中旗標、不啟動背景任務。
    #[tokio::test]
    async fn execute_at_skips_market_holiday() {
        let server = holiday_server("1991", holidays_1991()).await;
        let uri = server.uri();
        let result = with_origins(&[(TWSE, uri.as_str())], execute_at(at(1991, 10, 10))).await;
        result.expect("休市日直接結束");
        assert!(!IS_RUNNING.load(Ordering::SeqCst), "休市日不應啟動追蹤");
    }

    /// 休市日表的判斷：國慶日是休市日、「開始交易」列與其他日子不是。
    #[tokio::test]
    async fn is_holiday_matches_closed_dates_only() {
        let server = holiday_server("1991", holidays_1991()).await;
        let uri = server.uri();
        let day = |month, day| NaiveDate::from_ymd_opt(1991, month, day).expect("有效日期");
        let results = with_origins(&[(TWSE, uri.as_str())], async {
            (
                is_holiday(day(10, 10)).await.expect("查詢成功"),
                is_holiday(day(1, 4)).await.expect("查詢成功"),
                is_holiday(day(10, 11)).await.expect("查詢成功"),
            )
        })
        .await;
        assert_eq!(results, (true, false, false));
    }

    /// 已在追蹤中（執行中旗標已設）時，交易日再次觸發直接結束，不重複載入參考價、不再啟動背景任務。
    #[tokio::test]
    async fn execute_at_skips_when_tracking_is_already_running() {
        let server = holiday_server("1991", holidays_1991()).await;
        let uri = server.uri();
        let was_running = IS_RUNNING.swap(true, Ordering::SeqCst);
        let result = with_origins(&[(TWSE, uri.as_str())], execute_at(at(1991, 10, 11))).await;
        let still_running = IS_RUNNING.load(Ordering::SeqCst);
        IS_RUNNING.store(was_running, Ordering::SeqCst);

        result.expect("重複觸發直接結束");
        assert!(still_running, "旗標維持由原本的追蹤任務釋放");
    }

    /// 排程入口以現在時間執行：把今天設為休市日，不論哪天跑都不啟動追蹤。
    #[tokio::test]
    async fn execute_uses_current_time() {
        let today = Local::now().date_naive();
        let server = holiday_server(
            &today.year().to_string(),
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "ok", "date": format!("{}0101", today.year()), "queryYear": today.year(), "total": 1,
                "data": [[today.format("%Y-%m-%d").to_string(), "測試休市", ""]]
            })),
        )
        .await;
        let uri = server.uri();
        let result = with_origins(&[(TWSE, uri.as_str())], execute()).await;

        result.expect("今天休市直接結束");
        assert!(!IS_RUNNING.load(Ordering::SeqCst), "休市日不應啟動追蹤");
    }

    /// 休市日表回應無法解析時回錯誤，不能當成交易日照常啟動追蹤。
    #[tokio::test]
    async fn execute_at_fails_when_holiday_schedule_is_unreadable() {
        let server = holiday_server(
            "1991",
            ResponseTemplate::new(200).set_body_string("not json"),
        )
        .await;
        let uri = server.uri();
        let error = with_origins(&[(TWSE, uri.as_str())], execute_at(at(1991, 10, 11)))
            .await
            .expect_err("休市日表讀不到應回錯誤");
        assert!(
            error.to_string().contains("holiday schedule"),
            "錯誤要指出是休市日表：{error:#}"
        );
        assert!(
            !IS_RUNNING.load(Ordering::SeqCst),
            "讀不到休市日表時不應啟動追蹤"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_reconcile_target_prices() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    refresh_trace_targets_cache().await.unwrap();
    let result = reconcile_target_prices().await;
    assert!(result.is_ok());
}

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    let result = execute().await;
    assert!(result.is_ok());
}
