//! 條件選股回測 endpoint 整合測試：具名策略實盤追蹤。

use super::*;
use crate::app::ports::StrategyGateway;
use crate::interfaces::web::data_api::handlers::BacktestStrategyGateway;

const PRESET: &str = "79979 實盤追蹤";
const SYMBOLS: [&str; 3] = ["79979KA", "79979KB", "0050"];

fn track_day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1996, 5, d).expect("固定日期合法")
}

async fn cleanup_track(pool: &sqlx::PgPool) {
    let _ = sqlx::query("DELETE FROM strategy_rebalance WHERE preset_name = $1")
        .bind(PRESET)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM strategy_preset WHERE name = $1")
        .bind(PRESET)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1) AND "Date" BETWEEN $2 AND $3"#,
    )
    .bind(&SYMBOLS[..])
    .bind(track_day(1))
    .bind(track_day(31))
    .execute(pool)
    .await;
}

/// 策略存在但還沒有換股紀錄回空的 periods；有紀錄時逐期計算（未結束的最後一期到最新交易日）；
/// 查無策略回 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn strategy_track_follows_recorded_rebalances() {
    let Some(pool) = pool_or_skip("strategy_track_follows_recorded_rebalances").await else {
        return;
    };
    cleanup_track(pool).await;
    sqlx::query(
        "INSERT INTO strategy_preset (name, query, alert) VALUES ($1, 'from=1996-01-02', true)",
    )
    .bind(PRESET)
    .execute(pool)
    .await
    .expect("種入策略");
    let path = format!(
        "/api/v1/strategy-presets/{}/track",
        urlencoding::encode(PRESET)
    );
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["periods"], serde_json::json!([]));
    assert!(json["from"].is_null());

    // KA 100 → 110 → 121、KB 50 → 45；0050 100 → 102 → 105。
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES
           ('1996-05-01', '79979KA', 100), ('1996-05-15', '79979KA', 110), ('1996-05-31', '79979KA', 121),
           ('1996-05-01', '79979KB', 50), ('1996-05-15', '79979KB', 45),
           ('1996-05-01', '0050', 100), ('1996-05-15', '0050', 102), ('1996-05-31', '0050', 105)"#,
    )
    .execute(pool)
    .await
    .expect("種入日線");
    sqlx::query(
        "INSERT INTO strategy_rebalance
             (preset_name, date, query, hold_days, etf_weight, cash, market_filtered, symbols, names, weights)
         VALUES ($1, '1996-05-01', 'from=1996-01-02', 20, 50, 10, false,
                 ARRAY['79979KA', '79979KB'], ARRAY['甲', NULL], ARRAY[60, 30]::float8[]),
                ($1, '1996-05-15', 'from=1996-01-02', 20, 50, 0, false,
                 ARRAY['79979KA'], ARRAY['甲'], ARRAY[100]::float8[])",
    )
    .bind(PRESET)
    .execute(pool)
    .await
    .expect("種入換股紀錄");

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["from"], "1996-05-01");
    let first = &json["periods"][0];
    assert_eq!(first["end"], "1996-05-15");
    assert_eq!(first["ongoing"], false);
    assert_eq!(first["holdings"][0]["name"], "甲");
    assert!(first["holdings"][1]["name"].is_null());
    assert_eq!(first["holdings"][0]["return_percent"], 10.0);
    assert_eq!(first["holdings"][1]["return_percent"], -10.0);
    assert_eq!(first["strategy_return"], 3.0);
    assert_eq!(first["etf_return"], 2.0);
    assert_eq!(first["portfolio_return"], 2.5);
    let second = &json["periods"][1];
    assert_eq!(second["ongoing"], true);
    assert_eq!(second["strategy_return"], 10.0);
    assert!(json["cumulative_return"].is_number(), "{json}");

    // 換股通知的結算：05-15 換股時，上一期是 05-01～05-15，累積只算到 05-15。
    let gateway = BacktestStrategyGateway;
    let settlement = gateway
        .settlement(PRESET, track_day(15))
        .await
        .expect("結算成功")
        .expect("有上一期");
    assert_eq!(
        (settlement.start, settlement.end, settlement.periods),
        (track_day(1), track_day(15), 1)
    );
    assert_eq!(
        (
            settlement.strategy_return,
            settlement.etf_return,
            settlement.portfolio_return,
            settlement.cumulative_return,
            settlement.etf_cumulative_return,
        ),
        (Some(3.0), Some(2.0), Some(2.5), Some(2.5), Some(2.0))
    );
    assert!(
        gateway
            .settlement(PRESET, track_day(1))
            .await
            .expect("結算成功")
            .is_none()
    );

    let (status, _) =
        get("/api/v1/strategy-presets/79979%20%E4%B8%8D%E5%AD%98%E5%9C%A8/track").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup_track(pool).await;
}
