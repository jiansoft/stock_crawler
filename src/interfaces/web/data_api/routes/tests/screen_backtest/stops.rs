//! 條件選股回測 endpoint 整合測試：期間停損與移動停利。

use super::*;

/// 期間停損：BA 03-01 收盤 100、03-03 跌到 88（−12%）觸發 10% 停損，這期報酬改為 −12% 並扣賣出成本；
/// 下一期沒有日線的股票不檢查。移動停利同樣以還原收盤檢查；偏移穩健度共用日線快取；門檻超出範圍
/// 回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_stop_loss_and_trailing_stop() {
    let Some(pool) = seeded("screen_backtest_stop_loss_and_trailing_stop").await else {
        return;
    };
    sqlx::query(
        r#"UPDATE "DailyQuotes" SET "ClosingPrice" = 100 WHERE stock_symbol = '79979BA' AND "Date" = '1993-03-01'"#,
    )
    .execute(pool)
    .await
    .expect("設定 BA 收盤");
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES
           ('1993-03-02', '79979BA', 95), ('1993-03-03', '79979BA', 88), ('1993-03-04', '79979BA', 120)"#,
    )
    .execute(pool)
    .await
    .expect("種入 BA 收盤");

    let (status, json) = query("limit=2&stop_loss=10").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["stop_loss"], 10);
    assert!(json["trailing_stop"].is_null());
    // 03-01 BA −12%（停損）、BC 2%；03-08 BB、BC 沒有日線，照原本報酬。
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(-5.0)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    assert_eq!(json["periods"][0]["stopped"], 1);
    assert_eq!(json["periods"][1]["stopped"], 0);
    assert_eq!(json["summary"]["stop_exits"], 1);
    // 第一期：全部買進 0.1425%，停損賣出 0.5 × 0.88 付 0.4425%。
    assert_eq!(json["periods"][0]["return_after_costs"], -5.34);
    // 第二期：BA 已賣出不再付賣出成本；BC 期末 0.51 ÷ 0.95 減碼回一半、買進 BB 一半。
    assert_eq!(json["periods"][1]["return_after_costs"], 7.91);

    // 停損 15% 不觸發，結果與不設停損相同。
    let (_, loose) = query("limit=2&stop_loss=15").await;
    assert_eq!(loose["periods"][0]["avg_return"], 6.0);
    assert_eq!(loose["summary"]["stop_exits"], 0);

    // 移動停利 5%：03-02 收盤 95 已從買進價回落 5%，出場報酬 −5%。
    let (status, json) = query("limit=2&trailing_stop=5").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["trailing_stop"], 5);
    assert_eq!(json["periods"][0]["avg_return"], -1.5);

    // 偏移穩健度：各偏移共用日線快取，回應帶出停損設定。
    let (status, json) = get(
        "/api/v1/stocks/screen/backtest/offsets?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2&stop_loss=10",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["stop_loss"], 10);
    assert_eq!(json["offsets"][0]["summary"]["stop_exits"], 1);

    for (extra, message) in [
        ("stop_loss=0", "stop_loss 必須介於 1 至 50"),
        ("stop_loss=51", "stop_loss 必須介於 1 至 50"),
        ("trailing_stop=0", "trailing_stop 必須介於 1 至 50"),
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
        assert_eq!(json["error"], message);
    }
    cleanup(pool).await;
}
