//! 0050 在進場日停牌時，單次及期末回測均使用原定端點補算。

use super::*;

/// 暫停交易缺報酬時以最近有效收盤補算，次日開盤使用市場進場日與原定出場開盤。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要臨時 PostgreSQL")]
async fn screen_backtest_etf_suspension_fills_both_execution_models() {
    let Some(pool) = seeded("screen_backtest_etf_suspension").await else {
        return;
    };
    sqlx::query("DELETE FROM daily_forward_return WHERE stock_symbol='0050' AND date=$1")
        .bind(day(1))
        .execute(pool)
        .await
        .unwrap();
    // 1993 年的假資料只存在於臨時庫；以月內較早日期作為停牌前收盤。
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "OpeningPrice", "ClosingPrice") VALUES
        ('1993-03-01', '0050', 100, 100), ('1993-03-03', '0050', 104, 105),
        ('1993-03-08', '0050', 110, 112), ('1993-03-09', '0050', 120, 115),
        ('1993-03-15', '0050', 121, 122)"#,
    )
    .execute(pool)
    .await
    .unwrap();
    // close 缺報酬列亦應補算；next_open 進場日 03-02 沒有 0050 日線，不能跳到 03-03。
    sqlx::query("INSERT INTO daily_forward_return(date,stock_symbol,return_5d) VALUES ('1993-03-02','79979BA',10)")
        .execute(pool).await.unwrap();
    for (model, expected) in [("close", 12.0), ("next_open", 20.0)] {
        let (status, json) =
            query(&format!("limit=1&etf_weight=100&execution_price={model}")).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["periods"][0]["etf_return"], expected, "{json}");
        assert_eq!(json["summary"]["etf_missing_periods"], 0, "{json}");
        assert!(json["summary"]["daily_max_drawdown"].is_number(), "{json}");
        let (_, offsets) = get(&format!("/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&to=1993-03-15&hold_days=5&limit=1&etf_weight=100&execution_price={model}")).await;
        assert_eq!(
            offsets["offsets"][0]["summary"]["etf_missing_periods"], 0,
            "{offsets}"
        );
    }
    cleanup(pool).await;
}
