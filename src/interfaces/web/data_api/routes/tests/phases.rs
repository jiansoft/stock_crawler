//! Phase 1～4 endpoints 的資料庫語意整合測試。

use super::*;

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
