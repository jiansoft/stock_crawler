//! Phase 1 endpoints（個股報價、營收、財報、股利等）的資料庫語意整合測試。

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
