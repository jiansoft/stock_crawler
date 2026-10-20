//! 大盤趨勢狀態 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1994, 5, d).expect("固定日期合法")
}

/// 刪除種入的加權指數、市場廣度與估值。
async fn cleanup(pool: &sqlx::PgPool) {
    let _ = sqlx::query("DELETE FROM daily_market_valuation WHERE date BETWEEN $1 AND $2")
        .bind(day(1))
        .bind(day(31))
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM daily_stock_price_stats WHERE date BETWEEN $1 AND $2")
        .bind(day(1))
        .bind(day(31))
        .execute(pool)
        .await;
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
    // 市場廣度：05-03 站上 60 日線 3／4、05-04 沒有統計（取之前最近的 05-03）；上市列（id 2）不讀。
    for (d, market_id, above, below, high) in [(2, 0, 1, 3, 1), (3, 0, 3, 1, 2), (4, 2, 0, 9, 0)] {
        sqlx::query(
            "INSERT INTO daily_stock_price_stats
                 (date, stock_exchange_market_id, above_60_day_moving_average, below_60_day_moving_average,
                  above_20_day_moving_average, below_20_day_moving_average, new_52_week_high)
             VALUES ($1, $2, $3, $4, $3, $4, $5)",
        )
        .bind(day(d))
        .bind(market_id)
        .bind(above)
        .bind(below)
        .bind(high)
        .execute(pool)
        .await
        .expect("種入市場廣度");
    }
    // 估值：合計（id 0）05-02 本益比 15、05-03 為 16；05-04 只有上市列（id 2），不讀。
    for (d, market_id, pe, pb) in [(2, 0, "15", "1.5"), (3, 0, "16", "1.6"), (4, 2, "30", "3")] {
        sqlx::query(
            "INSERT INTO daily_market_valuation
                 (date, stock_exchange_market_id, median_pe, median_pb, pe_samples, pb_samples)
             VALUES ($1, $2, $3::numeric, $4::numeric, 100, 100)",
        )
        .bind(day(d))
        .bind(market_id)
        .bind(pe)
        .bind(pb)
        .execute(pool)
        .await
        .expect("種入估值");
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
    assert_eq!(
        json["breadth"],
        serde_json::json!({
            "date": "1994-05-03",
            "above_ma20_percent": 75.0,
            "above_ma60_percent": 75.0,
            "above_ma240_percent": null,
            "above_ma60_percent_5d_ago": null,
            "new_highs_5d": 3,
            "new_lows_5d": 0,
        })
    );

    // 估值取資料日（05-04）之前最近的合計列 05-03；只有兩天，5 日前為 null。
    assert_eq!(
        json["valuation"],
        serde_json::json!({
            "date": "1994-05-03",
            "median_pe": 16.0,
            "pe_percentile_5y": 100.0,
            "median_pe_5d_ago": null,
            "median_pb": 1.6,
            "pb_percentile_5y": 100.0,
        })
    );

    let (_, json) = get("/api/v1/market/regime?date=1994-05-03").await;
    assert_eq!(json["close"], 110.0);
    // 05-02 之前沒有加權指數；05-02 的廣度只有自己一天。
    let (_, json) = get("/api/v1/market/regime?date=1994-05-02").await;
    assert_eq!(json["breadth"]["above_ma60_percent"], 25.0);
    assert_eq!(json["breadth"]["new_highs_5d"], 1);
    assert_eq!(json["valuation"]["median_pe"], 15.0);
    // 沒有估值資料時為 null，不影響其他欄位。
    cleanup_valuation(pool).await;
    let (status, json) = get("/api/v1/market/regime?date=1994-05-03").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["valuation"].is_null(), "{json}");
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

/// 只刪除種入的估值，驗證沒有估值時的回應。
async fn cleanup_valuation(pool: &sqlx::PgPool) {
    let _ = sqlx::query("DELETE FROM daily_market_valuation WHERE date BETWEEN $1 AND $2")
        .bind(day(1))
        .bind(day(31))
        .execute(pool)
        .await;
}
