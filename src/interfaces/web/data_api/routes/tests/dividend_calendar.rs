//! `/market/dividend-calendar` 每筆除息／除權事件附歷年填息的整合測試（假代號，測完清除）。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

const SYMBOL: &str = "79979DC";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM dividend WHERE security_code = $1",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 2024-03-05 除息第 3 個交易日填息（近 5 年內的歷史）；2024-06-17 除息與 07-10 發放落在查詢區間：
/// 除息事件附 1／1 次、平均 3 日，發放事件不附。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn dividend_calendar_attaches_fill_history_to_ex_events() {
    let Some(pool) = pool_or_skip("dividend_calendar_attaches_fill_history_to_ex_events").await
    else {
        return;
    };
    cleanup(pool).await;
    for sql in [
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, $1, $1, 2, 1)"#,
        r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
           VALUES ($1, 2024, 2023, 'Q4', 2, 0, 2, '2024-03-05', '-', '2024-04-01', '-'),
                  ($1, 2024, 2024, 'Q1', 1, 0, 1, '2024-06-17', '-', '2024-07-10', '-')"#,
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
           VALUES ('2024-03-04', $1, 2024, 3, 4, 100, 100, 100, 100),
                  ('2024-03-05', $1, 2024, 3, 5, 97, 97, 97, 97),
                  ('2024-03-06', $1, 2024, 3, 6, 99, 99, 99, 99),
                  ('2024-03-07', $1, 2024, 3, 7, 100, 100, 100, 100)"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("寫入測試資料");
    }

    let (status, json) =
        get("/api/v1/market/dividend-calendar?from=2024-06-01&to=2024-07-31&limit=200").await;
    cleanup(pool).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let events: Vec<&serde_json::Value> = json["events"]
        .as_array()
        .expect("events 陣列")
        .iter()
        .filter(|event| event["stock_symbol"] == SYMBOL)
        .collect();
    let by_type = |kind: &str| {
        events
            .iter()
            .find(|event| event["event_type"] == kind)
            .unwrap_or_else(|| panic!("應有 {kind} 事件：{json}"))
    };
    let ex = by_type("ex_dividend");
    assert_eq!(ex["fill_history"]["events"], 1, "{ex}");
    assert_eq!(ex["fill_history"]["filled"], 1);
    assert_eq!(ex["fill_history"]["average_trading_days_to_fill"], 3.0);
    assert!(ex["fill_peer"].is_null(), "有自身紀錄時不附同組基準");
    let payable = by_type("cash_payable");
    assert!(payable["fill_history"].is_null() && payable["fill_peer"].is_null());
}
