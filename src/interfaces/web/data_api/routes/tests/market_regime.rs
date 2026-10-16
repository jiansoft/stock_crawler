//! 大盤趨勢狀態 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1994, 5, d).expect("固定日期合法")
}

/// 刪除種入的加權指數。
async fn cleanup(pool: &sqlx::PgPool) {
    let _ =
        sqlx::query(r#"DELETE FROM "index" WHERE category = 'TAIEX' AND "date" BETWEEN $1 AND $2"#)
            .bind(day(1))
            .bind(day(31))
            .execute(pool)
            .await;
}

/// 指定日期取之前最近的交易日、漲跌與均線資料不足時的欄位、只讀 TAIEX、參數驗證與查無資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn market_regime_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("market_regime_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    for (category, d, index) in [
        ("TAIEX", 2, 100),
        ("TAIEX", 3, 110),
        ("TAIEX", 4, 120),
        // 其他指數不列入。
        ("TPEx", 5, 999),
    ] {
        sqlx::query(r#"INSERT INTO "index" (category, "date", "index") VALUES ($1, $2, $3)"#)
            .bind(category)
            .bind(day(d))
            .bind(Decimal::from(index))
            .execute(pool)
            .await
            .expect("種入指數");
    }

    // 05-06 沒有 TAIEX：取之前最近的 05-04。
    let (status, json) = get("/api/v1/market/regime?date=1994-05-06").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1994-05-04");
    assert_eq!(json["close"], 120.0);
    assert_eq!(json["change"], 10.0);
    assert_eq!(json["change_percent"], 9.09);
    assert_eq!(json["alignment"], "mixed");
    assert_eq!(json["moving_averages"][0]["days"], 5);
    assert!(json["moving_averages"][0]["value"].is_null());
    assert!(json["days_on_side_of_ma60"].is_null());
    assert_eq!(
        (json["high_52_week"].as_f64(), json["low_52_week"].as_f64()),
        (Some(120.0), Some(100.0))
    );
    assert_eq!(json["from_low_percent"], 20.0);
    assert!(json["return_5d"].is_null());
    assert_eq!(json["recent_crosses"], serde_json::json!([]));

    let (_, json) = get("/api/v1/market/regime?date=1994-05-03").await;
    assert_eq!(json["close"], 110.0);
    let (status, _) = get("/api/v1/market/regime?date=1994-5-3").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = get("/api/v1/market/regime?date=1900-01-01").await;
    cleanup(pool).await;
    sqlx::query(r#"DELETE FROM "index" WHERE category = 'TPEx' AND "date" = $1"#)
        .bind(day(5))
        .execute(pool)
        .await
        .expect("清除 TPEx");
    assert_eq!(status, StatusCode::NOT_FOUND);
}
