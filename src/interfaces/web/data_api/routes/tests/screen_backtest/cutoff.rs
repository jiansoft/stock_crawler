//! 回測截止日不洩漏未來報酬、日頻風險走完整流程的資料庫語意整合測試。

use super::*;

/// 回測截止日不洩漏未來報酬（B02）：單日回測或換股日在迄日前不足持有天數時，不計入完整期數，
/// 不偷看迄日之後的價格，0050 與策略共用同一終點。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_cutoff_does_not_leak_future_returns() {
    let Some(pool) = seeded("screen_backtest_cutoff_does_not_leak_future_returns").await else {
        return;
    };
    // 03-01 當天有相對強度與 5 日報酬（BA 10、BC 2、0050 4），但迄日只查 03-01 單日：
    // 出場日 03-08 超過迄日，完整期數必須為 0，不可計入 03-08 的報酬，0050 與策略共用同一終點。
    let (status, json) =
        get("/api/v1/stocks/screen/backtest?from=1993-03-01&to=1993-03-01&hold_days=5&limit=2")
            .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["summary"]["periods"], 0);
    assert!(json["summary"]["total_return"].is_null());
    assert!(json["summary"]["total_return_after_costs"].is_null());
    assert!(json["summary"]["etf_total_return"].is_null());
    assert_eq!(periods(&json), []);
    // 目前持股列出 03-01 換股名單，但經過天數為 0，不使用 03-01 之後的價格。
    let current = &json["current"];
    assert!(!current.is_null());
    assert_eq!(current["date"], "1993-03-01");
    assert_eq!(current["days_elapsed"], 0);
    assert_eq!(current["days_to_next_rebalance"], 5);

    cleanup(pool).await;
}

/// 經真實 engine、權重、核心混合與彙總流程驗證日頻風險，避免僅測 helper 而漏掉呼叫順序。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn daily_risk_matches_actual_weighted_and_core_portfolio() {
    let pool = seeded("daily_risk_matches_actual_weighted_and_core_portfolio")
        .await
        .expect("隔離 DB 必須可用");
    for d in DAYS {
        for (symbol, value) in [
            ("79979BA", 30_000_000_i64),
            ("79979BC", 200_000_000),
            ("0050", 0),
        ] {
            let close = if symbol == "79979BA" && d == 2 {
                50
            } else {
                100
            };
            sqlx::query(r#"INSERT INTO "DailyQuotes"("Date",stock_symbol,"ClosingPrice","TradeValue") VALUES ($1,$2,$3,$4) ON CONFLICT("Date",stock_symbol) DO UPDATE SET "ClosingPrice"=EXCLUDED."ClosingPrice""#)
                .bind(day(d)).bind(symbol).bind(close).bind(value).execute(pool).await.unwrap();
        }
    }
    sqlx::query("UPDATE daily_forward_return SET return_5d=0 WHERE date=$1 AND stock_symbol IN ('79979BA','79979BC','0050')").bind(day(1)).execute(pool).await.unwrap();
    let path = "/api/v1/stocks/screen/backtest?from=1993-03-01&to=1993-03-08&hold_days=5&limit=2&weighting=trade_value&fee_discount=0";
    let (status, weighted) = get(path).await;
    assert_eq!(status, StatusCode::OK, "{weighted}");
    assert_eq!(weighted["summary"]["total_return"], 0.0);
    assert_eq!(weighted["summary"]["daily_max_drawdown"], -6.52);
    let (status, core) = get(&format!("{path}&etf_weight=100")).await;
    assert_eq!(status, StatusCode::OK, "{core}");
    assert_eq!(core["summary"]["daily_max_drawdown"], 0.0);
    let (status, stopped) = get(&format!("{path}&stop_loss=10")).await;
    assert_eq!(status, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["summary"]["total_return"], -6.52);
    assert_eq!(stopped["summary"]["daily_max_drawdown"], -6.52);
    assert_eq!(stopped["summary"]["daily_max_drawdown_after_costs"], -6.54);
    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol='79979BC'"#)
        .execute(pool)
        .await
        .unwrap();
    let (_, missing) = get(path).await;
    assert!(
        missing["summary"]["daily_max_drawdown"].is_null(),
        "{missing}"
    );
    cleanup(pool).await;
}
