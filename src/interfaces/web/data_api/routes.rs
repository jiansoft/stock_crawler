//! Data API 的路由註冊。
//!
//! `/api/v1` 下除健康檢查外一律套用 Bearer middleware；Swagger UI 與
//! OpenAPI JSON 不受驗證保護，方便內網服務直接產生 client。

use axum::{Router, middleware};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::openapi::ApiDoc;
use super::{auth, handlers};

/// 建立 `/api/v1` 路由與不受驗證保護的 Swagger/OpenAPI 文件入口。
pub(crate) fn router() -> Router {
    let protected = Router::new()
        .route(
            "/stocks/search",
            axum::routing::get(handlers::search_stocks),
        )
        .route(
            "/stocks/{symbol}/latest-quote",
            axum::routing::get(handlers::latest_quote),
        )
        .route(
            "/stocks/{symbol}/price-history",
            axum::routing::get(handlers::price_history),
        )
        .route(
            "/stocks/{symbol}/profile",
            axum::routing::get(handlers::stock_profile),
        )
        .route(
            "/stocks/{symbol}/realtime-snapshot",
            axum::routing::get(handlers::realtime_snapshot),
        )
        .route(
            "/stocks/{symbol}/monthly-revenues",
            axum::routing::get(handlers::monthly_revenues),
        )
        .route(
            "/stocks/{symbol}/financial-statements",
            axum::routing::get(handlers::financial_statements),
        )
        .route(
            "/stocks/{symbol}/dividends",
            axum::routing::get(handlers::dividend_history),
        )
        .route(
            "/stocks/{symbol}/valuation",
            axum::routing::get(handlers::stock_valuation),
        )
        .route(
            "/market/breadth",
            axum::routing::get(handlers::market_breadth),
        )
        .route(
            "/market/dividend-yield-ranking",
            axum::routing::get(handlers::dividend_yield_ranking),
        )
        .route(
            "/stocks/screen",
            axum::routing::get(handlers::screen_stocks),
        )
        .route(
            "/market/index-history",
            axum::routing::get(handlers::market_index_history),
        )
        .route(
            "/market/dividend-calendar",
            axum::routing::get(handlers::dividend_calendar),
        )
        .route(
            "/market/qfii-holding-ranking",
            axum::routing::get(handlers::qfii_holding_ranking),
        )
        .route(
            "/market/cagr-ranking",
            axum::routing::get(handlers::cagr_ranking),
        )
        .route(
            "/market/cagr-ranking/{stock_symbol}",
            axum::routing::get(handlers::cagr_by_symbol),
        )
        .layer(middleware::from_fn(auth::require_bearer_key));
    Router::new()
        .nest(
            "/api/v1",
            protected.route("/healthz", axum::routing::get(handlers::healthz)),
        )
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
}

#[cfg(test)]
mod tests {
    //! 不連資料庫的 API 基礎契約測試。
    //!
    //! 這些測試先驗證路由、驗證邊界與 OpenAPI 產物；資料庫查詢語意則由整合
    //! 測試環境驗證，避免單元測試因本機沒有 PostgreSQL 而失去可重現性。

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::router;

    /// 健康檢查必須免驗證，讓部署系統能在未持有 API key 時偵測存活狀態。
    #[tokio::test]
    async fn healthz_is_public() {
        let response = router()
            .oneshot(
                Request::get("/api/v1/healthz")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// 未帶 token 的受保護路徑必須在觸及資料庫前直接被拒絕。
    #[tokio::test]
    async fn protected_endpoint_rejects_missing_bearer_key() {
        let response = router()
            .oneshot(
                Request::get("/api/v1/stocks/search?query=2330")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 新增的三個歷史 endpoint 也必須受 Bearer 驗證保護，未帶 token 一律 401。
    #[tokio::test]
    async fn phase1_endpoints_reject_missing_bearer_key() {
        for path in [
            "/api/v1/stocks/2330/monthly-revenues",
            "/api/v1/stocks/2330/financial-statements",
            "/api/v1/stocks/2330/dividends",
        ] {
            let response = router()
                .oneshot(
                    Request::get(path)
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} 應回 401"
            );
        }
    }

    /// Phase 2 三個分析 endpoints 必須在 middleware 層拒絕未授權請求，
    /// 確保 401 發生在任何 SQL 查詢之前。
    #[tokio::test]
    async fn phase2_endpoints_reject_missing_bearer_key() {
        for path in [
            "/api/v1/stocks/2330/valuation",
            "/api/v1/market/breadth",
            "/api/v1/market/dividend-yield-ranking",
        ] {
            let response = router()
                .oneshot(
                    Request::get(path)
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} 應回 401"
            );
        }
    }

    /// Phase 3 選股 endpoint 必須先經 Bearer middleware，未授權請求不得執行
    /// 任何每股 LATERAL SQL。
    #[tokio::test]
    async fn phase3_endpoint_rejects_missing_bearer_key() {
        let response = router()
            .oneshot(
                Request::get("/api/v1/stocks/screen?market=twse")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Phase 1 endpoints 的資料庫語意整合測試（§3.2、§3.3）。
    ///
    /// 覆蓋三種語意：未知代號 → 404；已知代號但指定範圍無資料 → 200 空
    /// 陣列且 `data_as_of` 為 null；參數不合法 → 422。無資料庫時跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn phase1_endpoints_db_semantics() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(crate::infra::database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 phase1_endpoints_db_semantics：無資料庫連接");
            return;
        }
        // Auth middleware 讀環境變數 DATA_API_KEY；測試環境沒設定時自行
        // 補一組（--test-threads=1 下無資料競爭疑慮）。
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "phase1-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        // 以真實 router 發出帶 token 的請求並解析 JSON body。
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
                    .expect("body should be readable");
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("body should be JSON");
                (status, json)
            }
        };

        // 語意一：未知代號在三個 endpoint 都必須是 404。
        for path in [
            "/api/v1/stocks/NO_SUCH_SYMBOL/monthly-revenues",
            "/api/v1/stocks/NO_SUCH_SYMBOL/financial-statements",
            "/api/v1/stocks/NO_SUCH_SYMBOL/dividends",
        ] {
            let (status, _) = get(path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path} 未知代號應回 404");
        }

        // 語意二：已知代號 + 必然無資料的範圍 → 200 空陣列、data_as_of null。
        // 從 stocks 任取一檔存在的股票，避免測試依賴特定代號。
        let symbol: Option<(String,)> = sqlx::query_as("SELECT stock_symbol FROM stocks LIMIT 1")
            .fetch_optional(crate::infra::database::get_connection())
            .await
            .expect("stocks query should work");
        let Some((symbol,)) = symbol else {
            println!("跳過語意二：stocks 表無資料");
            return;
        };
        let (status, json) = get(&format!(
            "/api/v1/stocks/{symbol}/monthly-revenues?from=1990-01&to=1990-02"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["revenues"], serde_json::json!([]), "空範圍應回空陣列");
        assert_eq!(json["data_as_of"], serde_json::Value::Null);
        // 股利的「空範圍」不能假設任意年份沒資料（台股歷史夠久，1990 也可能
        // 有配息），改由資料庫找出該股票最早的股利所屬年度，取其前一年驗證。
        let min_year: Option<(Option<i32>,)> =
            sqlx::query_as("SELECT MIN(year_of_dividend) FROM dividend WHERE security_code = $1")
                .bind(&symbol)
                .fetch_optional(crate::infra::database::get_connection())
                .await
                .expect("dividend min-year query should work");
        let empty_year = match min_year.and_then(|(value,)| value) {
            // 最早年度的前一年必然無資料；早於等於 1990 的極端情形跳過此斷言。
            Some(min) if min > 1990 => Some(min - 1),
            Some(_) => None,
            // 這檔股票完全沒有股利資料，任何合法年份都應回空陣列。
            None => Some(1990),
        };
        if let Some(year) = empty_year {
            let (status, json) = get(&format!(
                "/api/v1/stocks/{symbol}/dividends?from_year={year}&to_year={year}"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json["dividends"],
                serde_json::json!([]),
                "無資料年份應回空陣列"
            );
            assert_eq!(json["data_as_of"], serde_json::Value::Null);
        }
        let (status, json) = get(&format!(
            "/api/v1/stocks/{symbol}/financial-statements?limit=1"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["statements"].is_array(), "statements 必須是陣列");
        // quarterly 不得混入年度 alias；all 則將 DB 空字串穩定輸出成 A，且
        // year/period 必須維持由新到舊。
        let (_, quarterly) = get(&format!(
            "/api/v1/stocks/{symbol}/financial-statements?period_type=quarterly&limit=40"
        ))
        .await;
        assert!(
            quarterly["statements"]
                .as_array()
                .expect("statements array")
                .iter()
                .all(|row| matches!(row["quarter"].as_str(), Some("Q1" | "Q2" | "Q3" | "Q4")))
        );
        let (_, all_periods) = get(&format!(
            "/api/v1/stocks/{symbol}/financial-statements?period_type=all&limit=40"
        ))
        .await;
        let statements = all_periods["statements"]
            .as_array()
            .expect("statements array");
        let period_rank = |quarter: &str| match quarter {
            "A" => 7,
            "H2" => 6,
            "H1" => 5,
            "Q4" => 4,
            "Q3" => 3,
            "Q2" => 2,
            "Q1" => 1,
            _ => 0,
        };
        for pair in statements.windows(2) {
            let left = (
                pair[0]["year"].as_i64().unwrap(),
                period_rank(pair[0]["quarter"].as_str().unwrap()),
            );
            let right = (
                pair[1]["year"].as_i64().unwrap(),
                period_rank(pair[1]["quarter"].as_str().unwrap()),
            );
            assert!(left >= right, "財報必須依年度與期間新到舊");
        }
        let empty_symbol: Option<String> = sqlx::query_scalar("SELECT s.stock_symbol FROM stocks s WHERE NOT EXISTS (SELECT 1 FROM financial_statement f WHERE f.security_code = s.stock_symbol) LIMIT 1")
            .fetch_optional(crate::infra::database::get_connection()).await.expect("empty financial symbol query");
        if let Some(empty_symbol) = empty_symbol {
            let (status, json) = get(&format!(
                "/api/v1/stocks/{empty_symbol}/financial-statements"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["statements"], serde_json::json!([]));
            assert!(json["data_as_of"].is_null());
        }

        // 語意三：參數不合法 → 422（在存在性檢查之前就擋下）。
        for path in [
            format!("/api/v1/stocks/{symbol}/monthly-revenues?from=2026-13"),
            format!("/api/v1/stocks/{symbol}/monthly-revenues?from=2026-06&to=2026-01"),
            format!("/api/v1/stocks/{symbol}/monthly-revenues?limit=121"),
            format!("/api/v1/stocks/{symbol}/financial-statements?period_type=monthly"),
            format!("/api/v1/stocks/{symbol}/financial-statements?limit=0"),
            format!("/api/v1/stocks/{symbol}/dividends?from_year=1889"),
            format!("/api/v1/stocks/{symbol}/dividends?from_year=2024&to_year=2020"),
            format!("/api/v1/stocks/{symbol}/dividends?limit=81"),
        ] {
            let (status, _) = get(&path).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} 應回 422");
        }
    }

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
            let (status, json) =
                get(&format!("/api/v1/market/breadth?market={market}&days=3")).await;
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
        let (status, json) =
            get("/api/v1/market/dividend-yield-ranking?industry_id=2147483647").await;
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

    /// Phase 4 三個市場輔助 endpoints 必須在 middleware 層拒絕未授權請求，
    /// 確保 401 發生在任何 SQL 查詢之前。
    #[tokio::test]
    async fn phase4_endpoints_reject_missing_bearer_key() {
        for path in [
            "/api/v1/market/index-history",
            "/api/v1/market/dividend-calendar",
            "/api/v1/market/qfii-holding-ranking",
        ] {
            let response = router()
                .oneshot(
                    Request::get(path)
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} 應回 401"
            );
        }
    }

    /// M4 兩個 endpoint 都必須在 middleware 層拒絕未授權請求。
    #[tokio::test]
    async fn cagr_endpoints_reject_missing_bearer_key() {
        for path in [
            "/api/v1/market/cagr-ranking",
            "/api/v1/market/cagr-ranking/2330",
        ] {
            let response = router()
                .oneshot(
                    Request::get(path)
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} 應回 401"
            );
        }
    }

    /// M4 參數驗證一律在觸及資料庫之前完成，因此此測試不需要 PostgreSQL。
    ///
    /// 最關鍵的是 `Y5`／`Y10` 搭配 `metric=price` 必須回 422：近十年每年皆有
    /// 134–216 檔股票配股，長期間忽略配股的低估幅度顯著，不能默默回答。
    #[tokio::test]
    async fn cagr_ranking_rejects_invalid_parameters_before_any_query() {
        // Auth middleware 讀環境變數 DATA_API_KEY；測試環境沒設定時自行
        // 補一組（CI 以 --test-threads=1 執行，無資料競爭疑慮）。
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "cagr-param-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        for path in [
            // 期間、口徑、排序鍵的白名單。
            "/api/v1/market/cagr-ranking?period=Y8",
            "/api/v1/market/cagr-ranking?period=y1",
            "/api/v1/market/cagr-ranking?metric=cash",
            "/api/v1/market/cagr-ranking?sort=return",
            // 長期間不提供純價格口徑。
            "/api/v1/market/cagr-ranking?period=Y5&metric=price",
            "/api/v1/market/cagr-ranking?period=Y10&metric=price",
            // 市場、產業、關鍵字、分頁與日期。
            "/api/v1/market/cagr-ranking?market=emerging",
            "/api/v1/market/cagr-ranking?stock_industry_id=0",
            "/api/v1/market/cagr-ranking?limit=0",
            "/api/v1/market/cagr-ranking?limit=201",
            "/api/v1/market/cagr-ranking?date=2026-8-6",
            // 個股端點共用同一組解析器。
            "/api/v1/market/cagr-ranking/2330?metric=cash",
            "/api/v1/market/cagr-ranking/2330?date=2026-08-6",
        ] {
            let response = router()
                .oneshot(
                    Request::get(path)
                        .header("Authorization", format!("Bearer {key}"))
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{path} 應回 422"
            );
        }
        // 短期間可用純價格口徑這條正向路徑會觸及資料庫，交由整合測試
        // （cagr_endpoints_db_semantics）驗證，此處刻意不發出該請求。
    }

    /// M4 兩個 endpoint 的真實資料庫語意整合測試（唯讀，不建立任何 fixture）。
    ///
    /// 驗證：全市場名次不受篩選影響且遞增、資料不足項目 `rank` 為 null 且
    /// 排在最後、金額欄位序列化為字串、涵蓋率分母正確，以及個股端點回傳
    /// 全部八個期間並依期間長度排序。M3 排程尚未產出資料時安全跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn cagr_endpoints_db_semantics() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(crate::infra::database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 cagr_endpoints_db_semantics：無資料庫連接");
            return;
        }
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "cagr-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        let get = |path: String| {
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
                    .expect("body should be readable");
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("body should be JSON");
                (status, json)
            }
        };

        // 先以「必然無資料的基準日」跑一次完整查詢：即使排程尚未產出任何
        // 結果，這條路徑仍會把排行 SQL（CTE、視窗函式、三個篩選、分頁）
        // 送進 PostgreSQL，語法或欄位錯誤會立刻現形。
        for path in [
            "/api/v1/market/cagr-ranking?date=1990-01-03&period=Y1",
            "/api/v1/market/cagr-ranking?date=1990-01-03&period=M3&sort=total_return",
            "/api/v1/market/cagr-ranking?date=1990-01-03&period=Y3&metric=price&market=twse",
            "/api/v1/market/cagr-ranking?date=1990-01-03&metric=reinvested&stock_industry_id=24",
            "/api/v1/market/cagr-ranking?date=1990-01-03&keyword=%E5%8F%B0%E7%A9%8D&offset=10",
            "/api/v1/market/cagr-ranking?date=1990-01-03&include_incomplete=false&limit=200",
        ] {
            let (status, json) = get(path.to_owned()).await;
            assert_eq!(status, StatusCode::OK, "{path} 應能在真實 schema 執行");
            assert_eq!(json["items"], serde_json::json!([]));
            assert_eq!(json["total"], 0);
            assert_eq!(json["coverage"]["universe"], 0);
            assert_eq!(json["base_date"], serde_json::Value::Null);
            assert_eq!(json["coverage"]["coverage_ratio"], "0.0000");
            assert_eq!(json["summary"]["positive_ratio"], "0.0000");
        }

        // 自行寫入一組計算結果再驗證有資料時的回應。早期版本改為「排程尚未
        // 產出結果就跳過」，於是 CI 從未跑過表頭、名次、涵蓋統計與個股端點 ——
        // 那正是這兩個 endpoint 的主體。資料一律用假代號與 1990-01-02，
        // 結束時清除。
        cagr_seed::cleanup().await;
        cagr_seed::seed().await;

        let (status, json) =
            get("/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&limit=50".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["period"], "Y1");
        assert_eq!(json["metric"], "total");
        assert_eq!(json["sort"], "cagr", "Y1 預設應以年化報酬率排序");
        assert_eq!(json["principal"], 10_000);
        assert_eq!(json["coverage"]["survivorship_note"], false);
        // 比率一律是字串（四位小數），不可是 JSON number。
        for pointer in ["/coverage/coverage_ratio", "/summary/positive_ratio"] {
            let value = json.pointer(pointer).expect("ratio 欄位應存在");
            assert!(value.is_string(), "{pointer} 必須序列化為字串");
        }

        // 表頭的期初日與年數取自可算項目，不得因為有資料不足的列就變成 null。
        assert_eq!(json["date"], "1990-01-02");
        assert_eq!(json["base_date"], "1989-01-03");
        assert!(json["years"].is_string());
        assert_eq!(json["coverage"]["universe"], 3);
        assert_eq!(json["coverage"]["counted"], 2);
        assert_eq!(json["coverage"]["incomplete"], 1);
        assert_eq!(json["total"], 3);

        let items = json["items"].as_array().expect("items array");
        assert_eq!(items.len(), 3);
        let mut previous_rank = 0_i64;
        let mut seen_incomplete = false;
        for item in items {
            match item["rank"].as_i64() {
                Some(rank) => {
                    assert!(!seen_incomplete, "資料不足的項目必須排在所有可算項目之後");
                    assert!(rank > previous_rank, "名次必須嚴格遞增");
                    assert!(item["data_complete"].as_bool().unwrap_or(false));
                    assert!(item["cagr_pct"].is_string(), "金額與比率必須是字串");
                    assert!(item["end_shares"].is_string());
                    previous_rank = rank;
                }
                None => {
                    seen_incomplete = true;
                    // 查得到但算不出來：列仍在，數值欄位為 null。
                    assert!(item["cagr_pct"].is_null());
                    assert!(item["end_value"].is_null());
                    assert!(item["stock_symbol"].is_string());
                }
            }
        }

        // 全市場名次：套用產業篩選後，名次仍是未篩選的完整市場排名。
        let (status, filtered) = get(format!(
            "/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&stock_industry_id={}&limit=50",
            cagr_seed::INDUSTRY
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            filtered["coverage"], json["coverage"],
            "涵蓋統計不隨畫面篩選變動"
        );
        assert!(
            filtered["total"].as_i64() <= json["total"].as_i64(),
            "篩選後的 total 不可大於未篩選"
        );
        let filtered_ranks: Vec<Option<i64>> = filtered["items"]
            .as_array()
            .expect("items array")
            .iter()
            .map(|item| item["rank"].as_i64())
            .collect();
        assert_eq!(
            filtered_ranks,
            items
                .iter()
                .map(|item| item["rank"].as_i64())
                .collect::<Vec<_>>(),
            "篩選後名次不得重編"
        );

        // 市場篩選對應到 stock_exchange_market_id：上櫃篩選不含這批上市假股票。
        let (status, tpex) =
            get("/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&market=tpex".to_owned())
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tpex["total"], 0);

        // 關鍵字比對代號與名稱。
        let (status, keyword) = get(format!(
            "/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&keyword={}",
            cagr_seed::TOP_SYMBOL
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(keyword["total"], 1);

        // include_incomplete = false 時資料不足的列整列消失。
        let (status, complete_only) = get(
            "/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&include_incomplete=false"
                .to_owned(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(complete_only["total"], 2);
        assert!(
            complete_only["items"]
                .as_array()
                .expect("items array")
                .iter()
                .all(|item| item["rank"].is_i64())
        );

        // Y10 + price 由 handler 在任何 SQL 之前擋下。
        let (status, _) =
            get("/api/v1/market/cagr-ranking?period=Y10&metric=price".to_owned()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Y10 的揭露旗標必須為 true，且長期間不提供純價格口徑（欄位為 null）。
        let (status, json) =
            get("/api/v1/market/cagr-ranking?date=1990-01-02&period=Y10&limit=1".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["coverage"]["survivorship_note"], true);

        // 個股端點：未知代號 404；已知代號回傳全部八個期間且由短至長。
        let (status, _) = get("/api/v1/market/cagr-ranking/NO_SUCH_SYMBOL".to_owned()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // 代號存在但該基準日沒有計算結果 —— 與「代號打錯」同為 404，但走的是
        // 另一條分支。
        let (status, _) = get(format!(
            "/api/v1/market/cagr-ranking/{}?date=1990-01-03",
            cagr_seed::TOP_SYMBOL
        ))
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, json) = get(format!(
            "/api/v1/market/cagr-ranking/{}?date=1990-01-02",
            cagr_seed::TOP_SYMBOL
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["stock_symbol"], cagr_seed::TOP_SYMBOL);
        assert_eq!(json["principal"], 10_000);
        let periods: Vec<&str> = json["items"]
            .as_array()
            .expect("items array")
            .iter()
            .map(|item| item["period"].as_str().expect("period string"))
            .collect();
        assert_eq!(
            periods,
            vec!["M3", "M6", "Y1", "Y1H", "Y2", "Y3", "Y5", "Y7", "Y10"],
            "個股端點必須回傳全部九個期間且依期間長度排序"
        );
        assert!(
            json["items"]
                .as_array()
                .expect("items array")
                .iter()
                .all(|item| item["rank"].is_null()),
            "個股端點的項目不應有名次"
        );

        // 指定 price 口徑時，長期間該口徑為 null 但列仍在。
        let (status, price) = get(format!(
            "/api/v1/market/cagr-ranking/{}?date=1990-01-02&metric=price",
            cagr_seed::TOP_SYMBOL
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        let long_period = price["items"]
            .as_array()
            .expect("items array")
            .iter()
            .find(|item| item["period"] == "Y10")
            .expect("Y10 應在清單中");
        assert!(long_period["cagr_pct"].is_null(), "長期間不提供純價格口徑");

        cagr_seed::cleanup().await;
    }

    /// `cagr_endpoints_db_semantics` 專用的測試資料。
    ///
    /// 代號一律以 `79979` 開頭（真實市場不存在），基準日固定 1990-01-02，
    /// 遠早於本功能上線，不會與正式資料互相干擾。
    ///
    /// 不加 `#[cfg(feature = "integration-tests")]`：使用它的測試函式在未開
    /// feature 時只是被標記為 `ignore`，本體仍要通過編譯。
    mod cagr_seed {
        use chrono::NaiveDate;
        use rust_decimal_macros::dec;

        use crate::domain::performance::{
            CagrPeriod, CagrRepository, StockCagr, entity::SimulationOutcome,
        };
        use crate::infra::database;
        use crate::infra::database::repository::performance::PgCagrRepository;

        /// 名次第一的假代號，同時用於個股端點與關鍵字篩選。
        pub(super) const TOP_SYMBOL: &str = "79979E1";
        const SECOND_SYMBOL: &str = "79979E2";
        const INCOMPLETE_SYMBOL: &str = "79979E3";
        /// 水泥工業，`stock_industry.sql` 已預先建立。
        pub(super) const INDUSTRY: i32 = 1;
        /// 上市（twse）；`cagr_market_id` 把 "twse" 對應到 2。
        const MARKET: i32 = 2;

        fn symbols() -> Vec<String> {
            vec![
                TOP_SYMBOL.to_string(),
                SECOND_SYMBOL.to_string(),
                INCOMPLETE_SYMBOL.to_string(),
            ]
        }

        fn base_date() -> NaiveDate {
            NaiveDate::from_ymd_opt(1990, 1, 2).expect("測試日期應合法")
        }

        fn record(symbol: &str, period: CagrPeriod, cagr: rust_decimal::Decimal) -> StockCagr {
            let outcome = SimulationOutcome {
                end_shares: dec!(100.5),
                cash_received: dec!(250.0),
                end_value: dec!(12000.0),
                total_return_pct: dec!(20.0),
                cagr_pct: cagr,
            };
            StockCagr {
                date: base_date(),
                stock_symbol: symbol.to_string(),
                period,
                base_date: NaiveDate::from_ymd_opt(1989, 1, 3),
                base_price: Some(dec!(100.0)),
                end_price: Some(dec!(119.4)),
                years: Some(dec!(1.0)),
                // 與計算層一致：長期間不提供純價格口徑。
                price: period.supports_price_metric().then_some(outcome),
                total: Some(outcome),
                reinvested: Some(outcome),
                first_quote_date: NaiveDate::from_ymd_opt(1988, 1, 4),
                shortfall_days: Some(0),
                data_complete: true,
                has_anomaly: false,
                dividend_events: 2,
            }
        }

        fn incomplete(symbol: &str, period: CagrPeriod) -> StockCagr {
            StockCagr {
                date: base_date(),
                stock_symbol: symbol.to_string(),
                period,
                base_date: None,
                base_price: None,
                end_price: None,
                years: None,
                price: None,
                total: None,
                reinvested: None,
                first_quote_date: NaiveDate::from_ymd_opt(1989, 6, 1),
                shortfall_days: None,
                data_complete: false,
                has_anomaly: true,
                dividend_events: 0,
            }
        }

        pub(super) async fn seed() {
            for (index, symbol) in symbols().iter().enumerate() {
                let _ = sqlx::query(
                    r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_industry_id,
                                           stock_exchange_market_id, "SuspendListing")
                       VALUES ($1, $2, $1, $3, $4, false)
                       ON CONFLICT (stock_symbol) DO UPDATE
                           SET "Name" = excluded."Name",
                               stock_industry_id = excluded.stock_industry_id,
                               stock_exchange_market_id = excluded.stock_exchange_market_id"#,
                )
                .bind(symbol)
                .bind(format!("測試股{}", index + 1))
                .bind(INDUSTRY)
                .bind(MARKET)
                .execute(database::get_connection())
                .await;
            }

            // 名次第一的個股寫滿八個期間，供個股端點驗證排序；
            // 另外兩檔只寫 Y1，構成「可算 2 檔 + 資料不足 1 檔」的母體。
            let mut records: Vec<StockCagr> = CagrPeriod::ALL
                .into_iter()
                .map(|period| record(TOP_SYMBOL, period, dec!(30)))
                .collect();
            records.push(record(SECOND_SYMBOL, CagrPeriod::Y1, dec!(10)));
            records.push(incomplete(INCOMPLETE_SYMBOL, CagrPeriod::Y1));

            PgCagrRepository::new()
                .save_batch(&records)
                .await
                .expect("寫入測試用 CAGR 結果");
        }

        pub(super) async fn cleanup() {
            let _ = sqlx::query("DELETE FROM stock_cagr WHERE stock_symbol = ANY($1)")
                .bind(symbols())
                .execute(database::get_connection())
                .await;
            let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
                .bind(symbols())
                .execute(database::get_connection())
                .await;
        }
    }

    /// Phase 4 三個 endpoints 的真實資料庫語意整合測試（§4.8–§4.10）。
    ///
    /// 覆蓋：參數不合法 → 422（含區間顛倒、區間超過 92 天、非法 enum、
    /// limit 超界）；查無資料 → 200 空陣列（三者皆無 404 語意）；行事曆
    /// 事件日期排序與無效日期標記不產生事件；QFII 排行的排除與排序規則。
    /// 無資料庫連線時安全跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn phase4_endpoints_db_semantics() {
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 phase4_endpoints_db_semantics：無資料庫連接");
            return;
        }
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "phase4-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        let get = |path: String| {
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
                    .expect("body should be readable");
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("body should be JSON");
                (status, json)
            }
        };

        // 語意一：參數不合法 → 422（在任何 SQL 之前擋下）。
        for path in [
            // §4.8 指數歷史。
            "/api/v1/market/index-history?from=2026-07-17&to=2026-01-01",
            "/api/v1/market/index-history?from=2026-7-1",
            "/api/v1/market/index-history?limit=0",
            "/api/v1/market/index-history?limit=366",
            // §4.9 行事曆：區間顛倒、超過 92 天、非法 enum、limit 超界。
            "/api/v1/market/dividend-calendar?from=2026-07-17&to=2026-07-01",
            "/api/v1/market/dividend-calendar?from=2026-01-01&to=2026-04-30",
            "/api/v1/market/dividend-calendar?event_type=cash",
            "/api/v1/market/dividend-calendar?limit=201",
            // §4.10 QFII 排行。
            "/api/v1/market/qfii-holding-ranking?market=emerging",
            "/api/v1/market/qfii-holding-ranking?sort_by=issued_share",
            "/api/v1/market/qfii-holding-ranking?industry_id=0",
            "/api/v1/market/qfii-holding-ranking?limit=51",
        ] {
            let (status, _) = get(path.to_owned()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} 應回 422");
        }

        // 語意二（§4.8）：index 表最早資料為 2018 年後，1900 年區間必然
        // 無資料 → 200 空陣列、data_as_of null（無 404 語意）。
        let (status, json) =
            get("/api/v1/market/index-history?from=1900-01-01&to=1900-12-31".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["points"], serde_json::json!([]));
        assert_eq!(json["data_as_of"], serde_json::Value::Null);

        // §4.8：正常查詢須依日期新到舊，data_as_of 為最新一筆日期。
        let (status, json) = get("/api/v1/market/index-history?limit=10".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        let points = json["points"].as_array().expect("points array");
        if let Some(first) = points.first() {
            assert_eq!(json["data_as_of"], first["date"]);
        }
        for pair in points.windows(2) {
            assert!(
                pair[0]["date"].as_str() > pair[1]["date"].as_str(),
                "指數歷史必須依日期由新到舊"
            );
        }

        // 語意三（§4.9）：1900 年代不可能有除權息事件 → 200 空陣列。
        let (status, json) =
            get("/api/v1/market/dividend-calendar?from=1900-01-01&to=1900-03-31".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["events"], serde_json::json!([]));
        assert_eq!(json["data_as_of"], serde_json::Value::Null);

        // §4.9：找一個實際有除息事件的區間驗證排序與日期合法性。以資料庫
        // 中最大的合法除息日為錨點，往前 30 天，保證區間內至少一筆事件。
        let anchor: Option<String> = sqlx::query_scalar(
            r#"SELECT MAX("ex-dividend_date1") FROM dividend
               WHERE "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$'"#,
        )
        .fetch_one(pool)
        .await
        .expect("anchor query should work");
        if let Some(anchor) = anchor {
            let to = chrono::NaiveDate::parse_from_str(&anchor, "%Y-%m-%d").expect("anchor date");
            let from = to - chrono::Days::new(30);
            let (status, json) = get(format!(
                "/api/v1/market/dividend-calendar?from={from}&to={to}&limit=200"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            let events = json["events"].as_array().expect("events array");
            assert!(!events.is_empty(), "錨點區間內至少應有一筆除息事件");
            for event in events {
                // 每筆事件日期都必須是合法日期且落在查詢區間內——這同時
                // 證明 `-`、`尚未公布` 等無效標記不會產生事件。
                let date = chrono::NaiveDate::parse_from_str(
                    event["event_date"].as_str().expect("event_date string"),
                    "%Y-%m-%d",
                )
                .expect("event_date 必須是合法日期");
                assert!((from..=to).contains(&date), "事件日期必須落在查詢區間");
                assert!(matches!(
                    event["event_type"].as_str(),
                    Some("ex_dividend" | "ex_rights" | "cash_payable" | "stock_payable")
                ));
                assert!(matches!(
                    event["quarter"].as_str(),
                    Some("A" | "H1" | "H2" | "Q1" | "Q2" | "Q3" | "Q4")
                ));
            }
            // 行事曆語意：event_date ASC、同日 stock_symbol ASC。
            for pair in events.windows(2) {
                let left = (
                    pair[0]["event_date"].as_str().unwrap(),
                    pair[0]["stock_symbol"].as_str().unwrap(),
                );
                let right = (
                    pair[1]["event_date"].as_str().unwrap(),
                    pair[1]["stock_symbol"].as_str().unwrap(),
                );
                assert!(left <= right, "行事曆必須依日期升冪、同日依代號升冪");
            }
            // event_type 過濾：單一類型查詢不得混入其他事件。
            let (status, json) = get(format!(
                "/api/v1/market/dividend-calendar?from={from}&to={to}&event_type=ex_dividend&limit=200"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                json["events"]
                    .as_array()
                    .expect("events array")
                    .iter()
                    .all(|event| event["event_type"] == "ex_dividend")
            );
        }

        // 語意四（§4.10）：查無資料的產業 → 200 空陣列；正常查詢驗證排除
        // 與兩種排序。data_as_of 固定 null（快照無列級日期，不可偽造）。
        let (status, json) =
            get("/api/v1/market/qfii-holding-ranking?industry_id=2147483647".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["stocks"], serde_json::json!([]));
        assert_eq!(json["data_as_of"], serde_json::Value::Null);
        for sort_by in ["percentage", "shares"] {
            let (status, json) = get(format!(
                "/api/v1/market/qfii-holding-ranking?sort_by={sort_by}&limit=50"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["data_as_of"], serde_json::Value::Null);
            let stocks = json["stocks"].as_array().expect("stocks array");
            for (index, stock) in stocks.iter().enumerate() {
                assert_eq!(stock["rank"], index as u64 + 1, "名次必須從一連續遞增");
                assert!(
                    matches!(stock["market_id"].as_i64(), Some(2 | 4)),
                    "all 只含上市與上櫃"
                );
                assert_ne!(stock["qfii_shares_held"], 0, "零持股必須被排除");
            }
            let metric = match sort_by {
                "percentage" => "qfii_share_holding_percentage",
                _ => "qfii_shares_held",
            };
            for pair in stocks.windows(2) {
                let left = pair[0][metric].as_f64().unwrap();
                let right = pair[1][metric].as_f64().unwrap();
                assert!(left >= right, "{sort_by} 必須由高到低");
                if left == right {
                    assert!(pair[0]["stock_symbol"].as_str() <= pair[1]["stock_symbol"].as_str());
                }
            }
        }
        // twse 過濾不可混入其他市場。
        let (status, json) =
            get("/api/v1/market/qfii-holding-ranking?market=twse".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            json["stocks"]
                .as_array()
                .expect("stocks array")
                .iter()
                .all(|stock| stock["market_id"] == 2)
        );
    }
}
