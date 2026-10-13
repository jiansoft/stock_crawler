//! 技術指標 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::stocks::{get, pool_or_skip};

const SYMBOL: &str = "79979TA";

/// 刪除種入的日報價與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 截止日、天數、排除無成交列、參數驗證與未知代號。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn technicals_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("technicals_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, $1, $1, 2, 1)"#,
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入 stocks");

    // 1989-03-01 起連續 30 天、收盤 100～129 逐日上漲；最後一天是無成交（收盤 0）的占位列。
    let first = NaiveDate::from_ymd_opt(1989, 3, 1).expect("固定日期合法");
    for i in 0..=30_u32 {
        let close = if i == 30 { 0 } else { 100 + i64::from(i) };
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice")
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(first + chrono::Days::new(u64::from(i)))
        .bind(SYMBOL)
        .bind(Decimal::from(close))
        .bind(Decimal::from(close + 1))
        .bind(Decimal::from((close - 1).max(0)))
        .execute(pool)
        .await
        .expect("種入日報價");
    }

    // 預設 20 天；收盤 0 的 03-31 不算，最新為 03-30。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/technicals")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1989-03-30");
    assert_eq!(json["days"], 20);
    assert_eq!(json["adjusted"], false);
    let points = json["points"].as_array().expect("points array");
    assert_eq!(points.len(), 20);
    assert_eq!(points[0]["date"], "1989-03-11");
    let last = &points[19];
    assert_eq!(last["close"], 129.0);
    assert_eq!(last["ma5"], 127.0);
    assert_eq!(last["ma20"], 119.5);
    assert!(last["ma60"].is_null());
    assert_eq!(last["rsi14"], 100.0);
    assert!(
        json["signals"]
            .as_array()
            .expect("signals array")
            .iter()
            .any(|signal| signal["indicator"] == "kd" && signal["kind"] == "overbought")
    );

    // 截止日 03-10、取 3 天：最新為 03-10，只有 10 天日線故沒有月線。
    let (status, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/technicals?date=1989-03-10&days=3"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1989-03-10");
    let points = json["points"].as_array().expect("points array");
    assert_eq!(points.len(), 3);
    assert_eq!(points[2]["ma10"], 104.5);
    assert!(points[2]["ma20"].is_null());

    // 截止日早於所有日線：200 但沒有資料。
    let (status, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/technicals?date=1989-01-01"
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["data_as_of"].is_null());
    assert!(json["points"].as_array().expect("points array").is_empty());

    for query in ["days=0", "days=121", "date=1989-3-1"] {
        let (status, _) = get(&format!("/api/v1/stocks/{SYMBOL}/technicals?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/technicals").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}
