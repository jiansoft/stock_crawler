//! `/market/dividend-fill-stats` 的整合測試：參數驗證與回應形狀（樣本計算見倉儲與領域測試）。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

/// `years` 超出 1–8 回 422；成功時固定回四個分組，由低到高。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn market_dividend_fill_stats_endpoint_shape() {
    if pool_or_skip("market_dividend_fill_stats_endpoint_shape")
        .await
        .is_none()
    {
        return;
    }
    for years in [0, 9] {
        let (status, _) = get(&format!("/api/v1/market/dividend-fill-stats?years={years}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "years={years}");
    }

    let (status, json) = get("/api/v1/market/dividend-fill-stats?years=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["years"], 1);
    assert_eq!(json["mature_trading_days"], 120);
    assert!(
        json["from"].as_str().is_some_and(|from| from.len() == 10),
        "{json}"
    );
    let buckets: Vec<&str> = json["buckets"]
        .as_array()
        .expect("buckets 陣列")
        .iter()
        .map(|bucket| bucket["bucket"].as_str().expect("bucket 代碼"))
        .collect();
    assert_eq!(
        buckets,
        ["below_3", "from_3_to_5", "from_5_to_7", "at_least_7"]
    );
    assert_eq!(json["buckets"][1]["label"], "3～5%");
}
