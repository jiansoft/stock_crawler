//! 條件選股回測 endpoint 整合測試：相對類 0050 基準的報酬歸因。

use super::*;

/// 報酬歸因：基準為平均成交金額最高的股票依成交金額加權。03-01 基準 BA（3,000 萬、10%）、BC（2 億、
/// 2%）都在產業 24，策略同樣持有兩檔（等權 6%），差額全是選股；03-08 基準只剩有報酬的 BA（3%），
/// 策略持有的 BB（8%）在基準沒有的產業，差額算交互。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_attribution_splits_excess_return() {
    let Some(pool) = seeded("screen_backtest_attribution_splits_excess_return").await else {
        return;
    };
    // 基準先取換股日當天有成交的股票：補上 BA 03-08 的日線（20 日平均仍為 3,000 萬元）。
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "TradeValue") VALUES ('1993-03-08', '79979BA', 30000000)"#,
    )
    .execute(pool)
    .await
    .expect("種入 BA 03-08 日線");
    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/attribution?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["benchmark_size"], 50);
    assert_eq!(json["rebalance_offset"], 0);
    let summary = &json["summary"];
    assert_eq!(summary["periods"], 2);
    assert_eq!(summary["strategy_return"], 14.0);
    // 03-01：(30 × 10 + 200 × 2) ÷ 230 ≈ 3.04；03-08：3。
    assert_eq!(summary["benchmark_return"], 6.04);
    assert_eq!(summary["etf_return"], 4.0);
    assert_eq!(summary["excess_return"], 7.96);
    assert_eq!(summary["allocation"], 0.0);
    assert_eq!(summary["selection"], 2.96);
    assert_eq!(summary["interaction"], 5.0);
    let industries = json["industries"].as_array().expect("industries array");
    assert_eq!(industries.len(), 2);
    assert_eq!(industries[0]["total"], 5.0);
    assert_eq!(industries[0]["avg_weight"], 50.0);
    assert_eq!(industries[0]["benchmark_avg_weight"], 0.0);
    assert_eq!(industries[1]["industry"], "半導體業");
    assert_eq!(industries[1]["selection"], 2.96);
    assert_eq!(industries[1]["benchmark_avg_weight"], 100.0);
    let years = json["years"].as_array().expect("years array");
    assert_eq!(years.len(), 1);
    assert_eq!(years[0]["year"], 1993);
    assert_eq!(years[0]["totals"]["excess_return"], 7.96);

    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/attribution?from=1993-03-01&to=1993-03-15&hold_days=5&rebalance_offset=5",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    // 起日之後沒有換股日：沒有期數也不查基準。
    let (status, json) =
        get("/api/v1/stocks/screen/backtest/attribution?from=1993-03-15&to=1993-03-15&hold_days=5")
            .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["summary"]["periods"], 0);
    assert!(json["summary"]["etf_return"].is_null());
    cleanup(pool).await;
}
