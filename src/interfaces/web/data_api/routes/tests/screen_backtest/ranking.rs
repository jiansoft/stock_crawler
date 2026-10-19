//! 條件選股回測 endpoint 整合測試：候選股排序與持股加權。

use super::*;

/// 流動性門檻、排序方式、0050 核心部位、持股加權與單檔權重上限。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_ranking_and_weighting() {
    let Some(pool) = seeded("screen_backtest_ranking_and_weighting").await else {
        return;
    };
    // 流動性門檻 50 百萬元：BA（20 日平均 3,000 萬）與沒有日線的 BB 不買，只剩 BC；
    // 03-08 的 BC 沒有報酬。目前持股 03-15 也只有 BC。
    let (status, json) = query("limit=2&min_avg_trade_value=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_avg_trade_value"], 50);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(2.0)),
            ("1993-03-08".to_owned(), 1, None),
        ]
    );
    assert_eq!(
        json["current"]["holdings"],
        serde_json::json!([{"symbol": "79979BC", "name": "79979BC", "weight": 100.0, "added": false, "return_to_date": null}])
    );
    // 門檻 30 百萬元：BA 剛好等於門檻算通過。
    let (_, json) = query("limit=2&min_avg_trade_value=30").await;
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BC"])
    );
    let (_, json) = query("limit=2").await;
    assert!(json["min_avg_trade_value"].is_null());
    for value in ["0", "100001"] {
        let (status, _) = query(&format!("min_avg_trade_value={value}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
    }

    // 排序方式：預設依相對強度（03-01 BA、03-08 BB）；依成交金額改選 BC（2 億元）。
    let (_, json) = query("limit=1").await;
    assert_eq!(json["rank_by"], "rs_score");
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB"])
    );
    let (status, json) = query("limit=1&rank_by=trade_value").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rank_by"], "trade_value");
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BC"])
    );
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BC"])
    );
    // 依成交金額且有門檻：BB 沒有日線不列入，BA 排在 BC 之後。
    let (_, json) = query("limit=3&rank_by=trade_value&min_avg_trade_value=30").await;
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BC", "79979BA"])
    );
    // 依投信連買：03-08 BC（1 天）排在相對強度較高的 BB 之前；沒設門檻時沒有籌碼資料的股票
    // 也照樣列入（03-01 三檔都有，連買天數都算 0、依相對強度排）。
    let (status, json) = query("limit=3&rank_by=trust_streak").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BC", "79979BB"])
    );
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BC", "79979BB", "79979BA"])
    );
    // 依外資連買：03-08 BB（2 天）在前，其餘依相對強度。
    let (_, json) = query("limit=3&rank_by=foreign_streak").await;
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB", "79979BC", "79979BA"])
    );
    let (status, _) = query("rank_by=volume").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // 0050 核心部位 50%：03-01 為 (6 + 4) / 2、03-08 0050 沒有資料算 0 → 8 / 2；100% 等於只持有 0050。
    let (status, json) = query("limit=2&etf_weight=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["etf_weight"], 50);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(5.0)),
            ("1993-03-08".to_owned(), 2, Some(4.0)),
        ]
    );
    let (_, json) = query("limit=2&etf_weight=100").await;
    assert_eq!(json["summary"]["total_return_after_costs"], 4.0);
    assert_eq!(json["summary"]["avg_turnover"], 0.0);
    let (_, json) = query("limit=2").await;
    assert_eq!(json["etf_weight"], 0);
    let (status, _) = query("etf_weight=101").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // 依成交金額加權：03-01 BA（3,000 萬）與 BC（2 億）為 (10×30 + 2×200) ÷ 230；03-08 只有 BB 有報酬。
    // 目前持股 BA、BB：BB 沒有日線，以 BA 的成交金額補，兩檔各半。平方根加權介於等權與成交金額之間。
    let (_, json) = query("limit=2").await;
    assert_eq!(json["weighting"], "equal");
    let (status, json) = query("limit=2&weighting=trade_value").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["weighting"], "trade_value");
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(3.04)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    assert_eq!(
        json["current"]["holdings"],
        serde_json::json!([
            {"symbol": "79979BA", "name": "79979BA", "weight": 50.0, "added": true, "return_to_date": null},
            {"symbol": "79979BB", "name": "79979BB", "weight": 50.0, "added": false, "return_to_date": null},
        ])
    );
    let (_, json) = query("limit=2&weighting=sqrt_trade_value").await;
    assert_eq!(json["periods"][0]["avg_return"], 4.23);
    let (_, json) = query("limit=2&weighting=trade_value&hold_min_rs_score=10").await;
    assert_eq!(json["periods"][0]["avg_return"], 3.04);
    let (status, _) = query("weighting=market_cap").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // 單檔上限：依成交金額加權時 BC（2 億）壓到 60%、BA 拿到 40%：10×0.4 + 2×0.6 = 5.2。
    // 等權上限 30%：兩檔各 30%、現金 40%；03-08 BC 沒有報酬，只算 BB 的 8 × 0.6。
    assert!(json["max_weight"].is_null());
    let (status, json) = query("limit=2&weighting=trade_value&max_weight=60").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_weight"], 60);
    assert_eq!(json["periods"][0]["avg_return"], 5.2);
    assert_eq!(json["current"]["cash"], 0.0);
    let (_, json) = query("limit=2&max_weight=30").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(3.6)),
            ("1993-03-08".to_owned(), 2, Some(4.8)),
        ]
    );
    assert_eq!(json["current"]["cash"], 40.0);
    assert_eq!(json["current"]["holdings"][0]["weight"], 30.0);
    for value in ["0", "101"] {
        let (status, _) = query(&format!("max_weight={value}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
    }

    cleanup(pool).await;
}
