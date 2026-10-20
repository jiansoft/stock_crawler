//! Phase 4 endpoints 的資料庫語意整合測試。

use super::*;

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
    let (status, json) = get("/api/v1/market/qfii-holding-ranking?market=twse".to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        json["stocks"]
            .as_array()
            .expect("stocks array")
            .iter()
            .all(|stock| stock["market_id"] == 2)
    );
}
