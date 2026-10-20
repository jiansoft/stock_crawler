//! Phase 2 endpoints 與 Phase 3 條件選股的資料庫語意整合測試。

use super::*;

/// Phase 2 三個 authenticated endpoints 的真實 SQL 欄位、型別、JOIN 與
/// 404／空資料／排序語意整合測試；不建立 fixture，避免污染共享資料庫。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn phase2_endpoints_db_semantics() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 phase2_endpoints_db_semantics：無資料庫連接");
        return;
    }
    let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "phase2-integration-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    });
    let get = |path: &str| {
        let path = path.to_owned();
        let key = key.clone();
        async move {
            let response = router()
                .oneshot(
                    Request::get(&path)
                        .header("Authorization", format!("Bearer {key}"))
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body readable");
            let json: serde_json::Value =
                serde_json::from_slice(&bytes).expect("body should be JSON");
            (status, json)
        }
    };

    let (status, _) = get("/api/v1/stocks/NO_SUCH_SYMBOL/valuation").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let symbol: Option<String> =
        sqlx::query_scalar("SELECT security_code FROM estimate ORDER BY date DESC LIMIT 1")
            .fetch_optional(pool)
            .await
            .expect("estimate symbol query");
    if let Some(symbol) = symbol {
        let (status, json) = get(&format!(
            "/api/v1/stocks/{symbol}/valuation?date=1900-01-01"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["valuation"].is_null());
        let (status, json) = get(&format!("/api/v1/stocks/{symbol}/valuation")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["valuation"]["stock_symbol"], symbol);
        assert_eq!(json["data_as_of"], json["valuation"]["date"]);
    }

    for market in ["all", "twse", "tpex"] {
        let (status, json) = get(&format!("/api/v1/market/breadth?market={market}&days=3")).await;
        if status == StatusCode::NOT_FOUND {
            continue;
        }
        assert_eq!(status, StatusCode::OK);
        let history = json["history"].as_array().expect("history array");
        assert!((1..=3).contains(&history.len()));
        assert_eq!(json["breadth"], history[0]);
        assert!(history.iter().all(|row| row["market"] == market));
    }
    let (status, _) = get("/api/v1/market/breadth?date=1900-01-01").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    for market in ["all", "twse", "tpex"] {
        let (status, json) = get(&format!(
            "/api/v1/market/dividend-yield-ranking?market={market}&limit=50"
        ))
        .await;
        if status == StatusCode::NOT_FOUND {
            continue;
        }
        assert_eq!(status, StatusCode::OK);
        let stocks = json["stocks"].as_array().expect("stocks array");
        for pair in stocks.windows(2) {
            let left = pair[0]["dividend_yield_percent"].as_f64().unwrap();
            let right = pair[1]["dividend_yield_percent"].as_f64().unwrap();
            assert!(left >= right);
            if left == right {
                assert!(pair[0]["stock_symbol"].as_str() <= pair[1]["stock_symbol"].as_str());
            }
        }
    }
    let (status, json) = get("/api/v1/market/dividend-yield-ranking?industry_id=2147483647").await;
    if status == StatusCode::OK {
        assert_eq!(json["stocks"], serde_json::json!([]));
    }
}

/// Phase 3 選股的真實資料庫整合測試。
///
/// 驗證空的 `all` 查詢在 SQL 前回 422，以及以 `twse` 作為有效條件時能
/// 執行每股最新資料查詢並維持固定 envelope。沒有資料庫連線時安全跳過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn phase3_screen_endpoint_db_semantics() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(crate::infra::database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 phase3_screen_endpoint_db_semantics：無資料庫連接");
        return;
    }
    let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "phase3-integration-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    });
    let get = |path: &'static str| {
        let key = key.clone();
        async move {
            let response = router()
                .oneshot(
                    Request::get(path)
                        .header("Authorization", format!("Bearer {key}"))
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body should be readable");
            let json: serde_json::Value =
                serde_json::from_slice(&bytes).expect("body should be JSON");
            (status, json)
        }
    };

    let (status, _) = get("/api/v1/stocks/screen").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, json) = get(
        "/api/v1/stocks/screen?market=twse&sort_by=valuation_percentage&sort_order=desc&limit=2",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "合法選股 SQL 應能在真實 schema 執行"
    );
    assert_eq!(json["data_as_of"], serde_json::Value::Null);
    assert!(json["stocks"].is_array());
    let stocks = json["stocks"].as_array().unwrap();
    assert!(stocks.len() <= 2);
    for stock in stocks {
        assert_eq!(stock["market_id"], 2, "twse 篩選不可混入其他市場");
        for field in ["valuation_date", "yield_date"] {
            if let Some(date) = stock[field].as_str() {
                assert!(chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok());
            }
        }
        if let Some(month) = stock["revenue_month"].as_str() {
            assert_eq!(month.len(), 7, "營收來源月份應為 YYYY-MM");
        }
        if let Some(period) = stock["financial_period"].as_str() {
            assert!(period.contains("-Q"), "財報來源期間應為 YYYY-Qn");
        }
    }
    for pair in stocks.windows(2) {
        match (
            pair[0]["valuation_percentage"].as_f64(),
            pair[1]["valuation_percentage"].as_f64(),
        ) {
            (Some(left), Some(right)) => assert!(left >= right),
            (None, Some(_)) => panic!("DESC NULLS LAST 不可把 null 放在有效值之前"),
            _ => {}
        }
    }
}
