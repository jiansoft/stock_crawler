//! 產業相對強度 endpoint 的資料庫語意整合測試。

use super::*;

/// 產業相對強度：成分股分數取自全體計分；檔數門檻、市場篩選；前期沒有資料時比較欄位為 null；
/// 參數錯誤 422、查無 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn industry_strength_aggregates_scores() {
    let Some(pool) = pool_or_skip("industry_strength_aggregates_scores").await else {
        return;
    };
    let dates = weekdays(242);
    let end = dates[dates.len() - 2];
    cleanup(pool).await;
    seed(pool, &dates).await;

    let (status, json) = get(&format!(
        "/api/v1/market/industry-strength?date={end}&min_stocks=1"
    ))
    .await;
    let (_, otc) = get(&format!(
        "/api/v1/market/industry-strength?date={end}&market=tpex&min_stocks=1&order=asc"
    ))
    .await;
    let (_, strict) = get(&format!(
        "/api/v1/market/industry-strength?date={end}&min_stocks=3"
    ))
    .await;
    let mut errors = Vec::new();
    for path in [
        "/api/v1/market/industry-strength?order=up",
        "/api/v1/market/industry-strength?min_stocks=0",
        "/api/v1/market/industry-strength?market=xx",
        "/api/v1/market/industry-strength?date=1980-01-01",
    ] {
        errors.push(get(path).await.0);
    }
    cleanup(pool).await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], end.to_string());
    assert!(json["prior_as_of"].is_null(), "{json}");
    assert_eq!(json["universe"], 2);
    let semi = &json["items"][0];
    assert_eq!(semi["industry"], "半導體業");
    assert_eq!(semi["stocks"], 2);
    // RA 99、RB 1 → 中位數 50。
    assert_eq!(semi["median_rs_score"], 50.0);
    assert_eq!(semi["strong_ratio"], 0.5);
    assert_eq!(semi["weak_ratio"], 0.5);
    assert_eq!(semi["leaders"][0]["stock_symbol"], "79979RA");
    assert!(semi["median_change"].is_null());

    assert_eq!(otc["order"], "asc");
    assert_eq!(otc["items"][0]["stocks"], 1);
    assert_eq!(otc["items"][0]["median_rs_score"], 1.0);
    assert_eq!(strict["items"].as_array().map(Vec::len), Some(0));

    assert_eq!(
        errors,
        [
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::NOT_FOUND,
        ]
    );
}
