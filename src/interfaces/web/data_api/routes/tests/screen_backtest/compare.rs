//! 條件選股回測 endpoint 整合測試：情境比較（`/stocks/screen/backtest/compare`）。

use super::*;

/// 情境依請求順序回傳；覆蓋參數與共用參數合併；滑價只影響扣成本後報酬；不合法的情境回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_compare_runs_each_scenario() {
    let Some(pool) = seeded("screen_backtest_compare_runs_each_scenario").await else {
        return;
    };
    let base =
        "/api/v1/stocks/screen/backtest/compare?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2";
    let (status, json) = get(&format!(
        "{base}&scenario=slippage_bps%3D0%26label%3D%E5%9F%BA%E6%BA%96&scenario=slippage_bps%3D50"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let scenarios = json["scenarios"].as_array().expect("scenarios");
    assert_eq!(scenarios.len(), 2);
    assert_eq!(scenarios[0]["label"], "基準");
    assert_eq!(scenarios[1]["label"], "slippage_bps=50");
    assert_eq!(scenarios[1]["slippage_bps"], 50);
    assert_eq!(scenarios[0]["execution_model"], "close");
    let net = |i: usize| scenarios[i]["summary"]["total_return_after_costs"].as_f64();
    assert!(net(1) < net(0), "{json}");
    assert_eq!(
        scenarios[0]["summary"]["total_return"], scenarios[1]["summary"]["total_return"],
        "扣成本前報酬不受滑價影響"
    );
    // 與單次回測結果相同。
    let (_, single) = query("limit=2&slippage_bps=50").await;
    assert_eq!(scenarios[1]["summary"], single["summary"]);
    assert!(
        json["disclaimer"]
            .as_str()
            .unwrap()
            .contains("不會自動下單")
    );

    for extra in [
        "scenario=slippage_bps%3D0",
        "scenario=a%3D1&scenario=preset%3Dx",
        "scenario=slippage_bps%3D0&scenario=slippage_bps%3D999",
        "scenario=slippage_bps%3D0&scenario=rebalance_offset%3D9",
    ] {
        let (status, body) = get(&format!("{base}&{extra}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {body}");
    }
    let (status, _) = get(&format!(
        "{base}&preset=%E4%B8%8D%E5%AD%98%E5%9C%A8&scenario=a%3D1&scenario=b%3D2"
    ))
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    cleanup(pool).await;
}
