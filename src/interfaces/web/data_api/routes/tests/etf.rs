//! `/etfs`、`/etfs/{symbol}` 的資料庫語意整合測試：直接寫入預先整理好的 ETF 表再查詢。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

/// 測試用假代號。
const SYMBOL: &str = "79979EC";
/// 比較用的第二檔假代號。
const OTHER: &str = "79979EC2";

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM etf_snapshot WHERE stock_symbol = $1",
        "DELETE FROM etf_performance WHERE stock_symbol = $1",
        "DELETE FROM etf_daily_nav WHERE stock_symbol = $1",
        "DELETE FROM etf_holding_history WHERE stock_symbol = $1",
    ] {
        for symbol in [SYMBOL, OTHER] {
            let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
        }
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

    // 比較：代號不分大小寫；其中一檔不是 ETF 時 404。
    let (status, json) = get("/api/v1/etfs/compare?symbols=79979ec,79979EC2").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
    sqlx::query(
        "INSERT INTO etf_snapshot (stock_symbol, name, market_id, strategy) VALUES ($1, '路由測試比較', 2, 'bond')",
    )
    .bind(OTHER)
    .execute(pool)
    .await
    .expect("寫入第二檔快照");
    let (status, json) = get("/api/v1/etfs/compare?symbols=79979ec,79979EC2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["items"].as_array().map(Vec::len), Some(2));
    assert_eq!(json["items"][0]["ret_1y"], 1.23);
    assert_eq!(
        json["overlaps"].as_array().map(Vec::len),
        Some(0),
        "沒有前十大不列重疊"
    );
    // 持股歷史：三期取兩期，最舊那期仍和更早一期比較。
    sqlx::query(
        "INSERT INTO etf_holding_history (stock_symbol, as_of, rank, holding_symbol, holding_name, weight)
         VALUES ($1, '1987-01-31', 1, '2330', '台積電', 50), ($1, '1987-02-28', 1, '2330', '台積電', 51),
                ($1, '1987-02-28', 2, '2317', '鴻海', 5), ($1, '1987-03-31', 1, '2330', '台積電', 52.345)",
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("寫入持股歷史");
    let (status, json) = get("/api/v1/etfs/79979ec/holdings/history?periods=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["name"], "路由測試債券");
    let periods = json["periods"].as_array().expect("periods 是陣列");
    assert_eq!(periods.len(), 2);
    assert_eq!(periods[0]["as_of"], "1987-03-31");
    assert_eq!(periods[0]["holdings"][0]["weight"], 52.35);
    assert_eq!(periods[0]["changes"]["removed"][0]["label"], "2317 鴻海");
    assert_eq!(periods[1]["changes"]["previous_as_of"], "1987-01-31");
    assert_eq!(periods[1]["changes"]["added"][0]["label"], "2317 鴻海");
    let (status, _) = get("/api/v1/etfs/79979XX/holdings/history").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in [
        "/api/v1/etfs/0050/holdings/history?periods=0",
        "/api/v1/etfs/0050/holdings/history?periods=25",
        "/api/v1/etfs?sort=name",
        "/api/v1/etfs?limit=0",
        "/api/v1/etfs/0050;?nav_days=1",
        "/api/v1/etfs/0050?nav_days=61",
        "/api/v1/etfs/compare?symbols=0050",
        "/api/v1/etfs/compare?symbols=0050,0050",
    ] {
        let (status, json) = get(bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad} {json}");
    }
    cleanup(pool).await;
}
