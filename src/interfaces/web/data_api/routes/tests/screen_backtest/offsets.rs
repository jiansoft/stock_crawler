//! 條件選股回測 endpoint 整合測試：換股日偏移穩健度與 0050 比例前緣。

use super::*;

/// 換股日偏移穩健度、依年度分布、0050 核心部位前緣與樣本內外分割。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_offsets_and_frontier() {
    let Some(pool) = seeded("screen_backtest_offsets_and_frontier").await else {
        return;
    };
    // 換股日偏移穩健度：持有 5 日跑偏移 0～4。只有偏移 0 的 03-01、03-08 有報酬，彙總與單跑
    // 偏移 0 相同；其他偏移沒有已滿的期數，不列入分布。
    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        (json["hold_days"].as_u64(), json["limit"].as_u64()),
        (Some(5), Some(2))
    );
    assert_eq!(json["etf_benchmark"], "0050");
    let offsets = json["offsets"].as_array().expect("offsets array");
    assert_eq!(
        offsets
            .iter()
            .map(|item| item["rebalance_offset"].as_u64().unwrap_or(99))
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    let (_, single) = query("limit=2").await;
    assert_eq!(offsets[0]["summary"], single["summary"]);
    assert_eq!(offsets[1]["summary"]["periods"], 0);
    let spread = &json["distribution"];
    let annualized = &single["summary"]["annualized_return_after_costs"];
    assert_eq!(spread["annualized_return_after_costs"]["min"], *annualized);
    assert_eq!(spread["annualized_return_after_costs"]["max"], *annualized);
    assert_eq!(spread["beat_etf_return_rate"], 1.0);
    assert_eq!(
        spread["etf_max_drawdown"]["median"],
        single["summary"]["etf_max_drawdown"]
    );
    assert_eq!(spread["volatility"]["max"], single["summary"]["volatility"]);
    // 依年度：只有偏移 0 有期數，1993 年的分布就是單次回測的年度彙總。
    assert_eq!(
        (
            json["rank_by"].as_str(),
            json["etf_weight"].as_u64(),
            json["weighting"].as_str()
        ),
        (Some("rs_score"), Some(0), Some("equal"))
    );
    let years = json["years"].as_array().expect("years array");
    assert_eq!(years.len(), 1, "{years:?}");
    assert_eq!(
        (years[0]["year"].as_i64(), years[0]["offsets"].as_i64()),
        (Some(1993), Some(1))
    );
    assert_eq!(
        years[0]["return_after_costs"]["median"],
        single["years"][0]["total_return_after_costs"]
    );
    assert_eq!(
        years[0]["etf_return"]["median"],
        single["years"][0]["etf_total_return"]
    );
    // 0050 前緣：每個偏移只查一次，比例 0 的分布與偏移穩健度相同，100 等於只持有 0050。
    let (status, frontier) = get(
        "/api/v1/stocks/screen/backtest/frontier?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2&etf_weight=40",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{frontier}");
    assert_eq!(frontier["offsets"], 5);
    assert_eq!(frontier["weighting"], "equal");
    let weights = frontier["weights"].as_array().expect("weights array");
    assert_eq!(
        weights
            .iter()
            .map(|point| point["etf_weight"].as_u64().unwrap_or(999))
            .collect::<Vec<_>>(),
        [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
    );
    assert_eq!(weights[0]["distribution"], json["distribution"]);
    let full = &weights[10]["distribution"];
    assert_eq!(
        full["annualized_return_after_costs"]["median"],
        full["etf_annualized_return"]["median"]
    );
    assert!(frontier["best_sharpe_weight"].is_u64() || frontier["best_sharpe_weight"].is_null());
    assert!(frontier["best_return_weight"].is_u64(), "{frontier}");
    let (status, _) = get("/api/v1/stocks/screen/backtest/frontier?hold_days=10").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(json["split"].is_null() && json["before_split"].is_null());
    assert!(frontier["best_sharpe_weight_before_split"].is_null());
    assert!(weights[0]["after_split"].is_null());

    // 分割日 03-08：03-01 那期在分割日才出場，須排除，03-08 為樣本外。
    let (status, halves) = get(
        "/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2&split=1993-03-08",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{halves}");
    assert_eq!(halves["split"], "1993-03-08");
    assert_eq!(halves["distribution"], json["distribution"]);
    // 跨界期數排除後樣本內為空，樣本外仍有完整期數。
    let before = &halves["before_split"]["annualized_return_after_costs"];
    let after = &halves["after_split"]["annualized_return_after_costs"];
    assert!(before.is_null(), "{halves}");
    assert!(after["median"].is_number(), "{halves}");
    // 每個偏移揭露排除的跨界期數；沒有分割日時為 0。
    let excluded = |json: &serde_json::Value| -> Vec<u64> {
        json["offsets"]
            .as_array()
            .expect("offsets array")
            .iter()
            .map(|run| {
                run["excluded_cross_boundary_periods"]
                    .as_u64()
                    .unwrap_or(99)
            })
            .collect()
    };
    assert!(excluded(&halves).contains(&1), "{halves}");
    assert!(excluded(&json).iter().all(|&count| count == 0), "{json}");
    let (status, frontier) = get(
        "/api/v1/stocks/screen/backtest/frontier?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2&split=1993-03-08",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{frontier}");
    assert_eq!(frontier["split"], "1993-03-08");
    assert_eq!(
        frontier["weights"][0]["after_split"], halves["after_split"],
        "比例 0 的樣本外分布與偏移穩健度相同"
    );
    assert!(
        frontier["best_sharpe_weight_before_split"].is_u64()
            || frontier["best_sharpe_weight_before_split"].is_null()
    );
    for split in ["1993-03-01", "1993-03-16", "1993/03/08"] {
        let (status, json) = get(&format!(
            "/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&to=1993-03-15&hold_days=5&split={split}"
        ))
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{split}: {json}");
    }
    // 參數驗證與單次回測相同；rebalance_offset 不使用。
    for (path, status) in [
        ("hold_days=5&rebalance_offset=9", StatusCode::OK),
        ("hold_days=10", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("to=1900-01-01", StatusCode::NOT_FOUND),
    ] {
        let (actual, json) = get(&format!(
            "/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&{path}"
        ))
        .await;
        assert_eq!(actual, status, "{path}: {json}");
    }

    cleanup(pool).await;
}
