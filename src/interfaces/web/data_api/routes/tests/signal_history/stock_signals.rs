//! 個股訊號歷史 endpoint 的資料庫語意整合測試。

use super::*;

/// 由新到舊、附報價與之後報酬；篩選、limit 與 total；查無訊號、未知代號與參數驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_signals_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("stock_signals_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    let (status, json) = get("/api/v1/stocks/79979PA/signals?date=1991-10-03&days=5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["stock_symbol"], "79979PA");
    assert_eq!(json["from"], "1991-09-29");
    assert_eq!(json["to"], "1991-10-03");
    assert_eq!(json["adjusted"], true);
    assert_eq!(json["total"], 3);
    let items: Vec<(String, String)> = json["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| {
            (
                item["date"].as_str().unwrap_or_default().to_owned(),
                item["indicator"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        items,
        [
            ("1991-10-02".to_owned(), "kd".to_owned()),
            ("1991-10-01".to_owned(), "kd".to_owned()),
            ("1991-10-01".to_owned(), "52_week".to_owned()),
        ]
    );
    let first_day = &json["items"][1];
    assert_eq!(first_day["closing_price"], 50.0);
    assert_eq!(first_day["change_percent"], 1.5);
    assert_eq!(first_day["return_5d"], 4.0);
    assert_eq!(first_day["return_20d"], 10.0);
    assert!(first_day["return_60d"].is_null());
    assert!(first_day["return_120d"].is_null());
    assert_eq!(first_day["return_240d"], 30.0);
    assert_eq!(first_day["description"], "79979PA golden_cross");
    assert!(json["items"][0]["closing_price"].is_null());

    let (_, json) = get("/api/v1/stocks/79979PA/signals?date=1991-10-03&limit=1").await;
    assert_eq!(json["total"], 3);
    assert_eq!(json["items"].as_array().map(Vec::len), Some(1));
    let (_, json) =
        get("/api/v1/stocks/79979PA/signals?date=1991-10-03&kind=new_high&indicator=52_week").await;
    assert_eq!(json["total"], 1);
    let (status, json) = get("/api/v1/stocks/79979PA/signals?date=1991-10-03&days=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 0);
    assert_eq!(json["items"].as_array().map(Vec::len), Some(0));

    for (path, status) in [
        ("/api/v1/stocks/79979ZZ/signals", StatusCode::NOT_FOUND),
        (
            "/api/v1/stocks/79979PA/signals?days=0",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?days=731",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?limit=0",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?kind=bad",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/stocks/79979PA/signals?date=1991/10/03",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (actual, json) = get(path).await;
        assert_eq!(actual, status, "{path}: {json}");
    }

    cleanup(pool).await;
}
