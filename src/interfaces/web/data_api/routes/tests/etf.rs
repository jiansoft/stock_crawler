//! `/etfs`、`/etfs/{symbol}` 的資料庫語意整合測試：直接寫入預先整理好的 ETF 表再查詢。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

/// 測試用假代號。
const SYMBOL: &str = "79979EC";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM etf_snapshot WHERE stock_symbol = $1",
        "DELETE FROM etf_performance WHERE stock_symbol = $1",
        "DELETE FROM etf_daily_nav WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn etf_endpoints_db_semantics() {
    let Some(pool) = pool_or_skip("etf_endpoints_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    sqlx::query(
        "INSERT INTO etf_snapshot (stock_symbol, name, market_id, strategy, dividend_frequency, trade_value, premium_rate)
         VALUES ($1, '路由測試債券', 4, 'bond', 'monthly', 123, -0.456)",
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("寫入快照");
    sqlx::query(
        "INSERT INTO etf_performance (stock_symbol, calc_date, ret_1y) VALUES ($1, '1987-03-05', 1.234)",
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("寫入績效");
    sqlx::query(
        "INSERT INTO etf_daily_nav (date, stock_symbol, nav, market_price, premium_rate)
         VALUES ('1987-03-04', $1, 10, 9.9, -1), ('1987-03-05', $1, 10, 10.1, 1)",
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("寫入淨值");

    let (status, json) =
        get("/api/v1/etfs?keyword=79979EC&strategy=bond&market=tpex&frequency=monthly&sort=premium_rate&order=asc").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 1);
    assert_eq!(
        (json["sort"].clone(), json["order"].clone()),
        ("premium_rate".into(), "asc".into())
    );
    assert_eq!(json["items"][0]["market"], "tpex");
    assert_eq!(json["items"][0]["premium_rate"], -0.46);

    let (status, json) = get("/api/v1/etfs/79979EC?nav_days=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["item"]["name"], "路由測試債券");
    assert_eq!(json["performance"]["ret_1y"], 1.23);
    assert!(
        json["profile"].is_null() && json["holdings_as_of"].is_null(),
        "{json}"
    );
    assert_eq!(json["navs"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["navs"][0]["date"], "1987-03-05");

    let (status, _) = get("/api/v1/etfs/79979XX").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in [
        "/api/v1/etfs?sort=name",
        "/api/v1/etfs?limit=0",
        "/api/v1/etfs/0050;?nav_days=1",
        "/api/v1/etfs/0050?nav_days=61",
    ] {
        let (status, json) = get(bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad} {json}");
    }
    cleanup(pool).await;
}
