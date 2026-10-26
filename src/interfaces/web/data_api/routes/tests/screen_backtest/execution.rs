//! 條件選股回測 endpoint 整合測試：次日開盤成交（`execution_price=next_open`）與滑價（`slippage_bps`）。

use super::*;

/// 種入 BA 次日（03-02）起的 5 日報酬與 03-02、03-09 的開收盤：03-02 開 100 收 110、03-09 收 121
/// 開 133.1，收盤到收盤 +10%，開盤到開盤 +33.1%。
async fn seed_next_open(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO daily_forward_return (date, stock_symbol, return_5d) VALUES ($1, '79979BA', 10)",
    )
    .bind(day(2))
    .execute(pool)
    .await
    .expect("種入次日報酬");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "OpeningPrice", "ClosingPrice") VALUES
           ('1993-03-02', '79979BA', 100, 110), ('1993-03-09', '79979BA', 133.1, 121)"#,
    )
    .execute(pool)
    .await
    .expect("種入開收盤");
}

/// 次日開盤成交：03-01 收盤後才確定選股（RS 看 03-01），03-02 開盤買進、03-09 開盤賣出，報酬以
/// 開盤到開盤計；03-08 換股的出場日落在 03-15 之後，視為未完成。不支援停損；日頻回撤不小於期末口徑；
/// 參數超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_next_open_trades_at_following_open() {
    let Some(pool) = seeded("screen_backtest_next_open_trades_at_following_open").await else {
        return;
    };
    seed_next_open(pool).await;

    let (status, close) = query("limit=1&min_rs_score=85").await;
    assert_eq!(status, StatusCode::OK, "{close}");
    assert_eq!(close["execution_model"], "close");
    assert_eq!(close["slippage_bps"], 0);
    assert_eq!(
        close["periods"][0]["avg_return"], 10.0,
        "收盤版沿用換股日報酬"
    );

    let (status, json) = query("limit=1&min_rs_score=85&execution_price=next_open").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["execution_model"], "next_open");
    let first = &json["periods"][0];
    assert_eq!(first["date"], "1993-03-01");
    // 03-02 的分數全是 50；若誤用進場日的 RS，min_rs_score=85 會選不到任何股票。
    assert_eq!(first["symbols"], serde_json::json!(["79979BA"]));
    assert_eq!(first["exit_date"], "1993-03-09");
    let ret = first["avg_return"].as_f64().expect("有報酬");
    assert!((ret - 33.1).abs() < 0.01, "開盤到開盤報酬：{ret}");
    assert_eq!(
        json["periods"].as_array().map(Vec::len),
        Some(1),
        "03-08 那期在 03-15 前出不了場：{json}"
    );
    // 日頻回撤以開盤為進出場端點；幅度不小於期末口徑。
    let daily = json["summary"]["daily_max_drawdown_after_costs"].as_f64();
    let period_end = json["summary"]["period_end_max_drawdown_after_costs"]
        .as_f64()
        .expect("期末回撤");
    if let Some(daily) = daily {
        assert!(daily <= period_end + 1e-9, "{daily} vs {period_end}");
    }

    for extra in [
        "execution_price=vwap",
        "execution_price=next_open&stop_loss=10",
        "execution_price=next_open&trailing_stop=10",
        "slippage_bps=201",
    ] {
        let (status, body) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {body}");
    }
    cleanup(pool).await;
}

/// 滑價越大，扣成本後報酬越低；扣成本前報酬不變。偏移穩健度也揭露成交假設。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_slippage_lowers_after_cost_returns() {
    let Some(pool) = seeded("screen_backtest_slippage_lowers_after_cost_returns").await else {
        return;
    };
    let mut previous: Option<f64> = None;
    let mut gross = None;
    for bps in [0, 20, 50, 200] {
        let (status, json) = query(&format!("limit=2&slippage_bps={bps}")).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["slippage_bps"], bps);
        let net = json["summary"]["total_return_after_costs"]
            .as_f64()
            .expect("有扣成本後報酬");
        if let Some(previous) = previous {
            assert!(net < previous, "滑價 {bps} bps 應更低：{net} vs {previous}");
        }
        previous = Some(net);
        let total = json["summary"]["total_return"].as_f64();
        assert!(
            gross.is_none() || gross == Some(total),
            "扣成本前不受滑價影響"
        );
        gross = Some(total);
    }
    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&to=1993-03-15&hold_days=5&slippage_bps=20&execution_price=next_open",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["execution_model"], "next_open");
    assert_eq!(json["slippage_bps"], 20);
    cleanup(pool).await;
}
