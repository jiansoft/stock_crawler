//! 條件選股回測 endpoint 整合測試：具名策略與策略換股通知 gateway。

use super::*;

/// 策略換股通知的 adapter：與回測端點相同的條件與目前持股，拆成新買進、續抱與賣出；條件錯誤、
/// 缺 `from`、最後一期已滿或籌碼還沒入庫時回錯誤或 `None`。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn strategy_gateway_reports_latest_rebalance() {
    let Some(pool) = seeded("strategy_gateway_reports_latest_rebalance").await else {
        return;
    };
    use crate::app::ports::{StrategyGateway, StrategyHolding};
    use crate::interfaces::web::data_api::BacktestStrategyGateway;

    let gateway = BacktestStrategyGateway;
    let holding = |symbol: &str| StrategyHolding {
        symbol: symbol.to_owned(),
        name: Some(symbol.to_owned()),
        weight: 50.0,
        return_to_date: None,
    };
    let rebalance = gateway
        .latest_rebalance("from=1993-03-01&to=1993-03-15&hold_days=5&limit=2&etf_weight=20")
        .await
        .expect("回測成功")
        .expect("03-15 還沒滿持有天數");
    assert_eq!(rebalance.date, day(15));
    assert_eq!((rebalance.hold_days, rebalance.etf_weight), (5, 20));
    assert!(!rebalance.market_filtered);
    assert_eq!(rebalance.added, [holding("79979BA")]);
    assert_eq!(rebalance.kept, [holding("79979BB")]);
    assert_eq!(rebalance.exited, [holding("79979BC")]);

    // 03-12 時最後換股日 03-08 已有報酬，沒有目前持股。
    let none = gateway
        .latest_rebalance("from=1993-03-01&to=1993-03-12&hold_days=5&limit=2")
        .await
        .expect("回測成功");
    assert!(none.is_none());

    for (query, expected) in [
        ("to=1993-03-15&hold_days=5", "必須指定 from"),
        ("from=1993-03-01&to=1993-03-15&hold_days=7", "HTTP 422"),
        (
            "from=1993-03-01&to=1993-03-15&hold_days=5&rebalance_offset=5",
            "rebalance_offset",
        ),
        (
            "from=1993-03-01&to=1993-03-15&hold_days=5&limit=x",
            "無法解析",
        ),
        // 03-15 沒有籌碼資料：外資連買條件看不到當天。
        (
            "from=1993-03-01&to=1993-03-15&hold_days=5&min_foreign_streak=1",
            "籌碼資料尚未入庫",
        ),
    ] {
        let error = gateway
            .latest_rebalance(query)
            .await
            .expect_err(query)
            .to_string();
        assert!(error.contains(expected), "{query}: {error}");
    }

    cleanup(pool).await;
}

/// 管理介面的具名策略 API；`name` 為空時打清單路徑。
async fn admin(method: &str, name: &str, body: &str) -> (StatusCode, serde_json::Value) {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let response = crate::interfaces::web::backfill_admin::router()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(if name.is_empty() {
                    "/api/strategy-presets".to_owned()
                } else {
                    format!("/api/strategy-presets/{}", urlencoding::encode(name))
                })
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

/// 具名策略：管理介面存入（條件驗證不過回 422）、清單端點列出、回測以 `preset` 引用且請求參數覆蓋
/// 存的值、查無策略回 404、刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn strategy_presets_are_saved_listed_and_referenced() {
    let Some(pool) = seeded("strategy_presets_are_saved_listed_and_referenced").await else {
        return;
    };
    let name = "79979 測試策略";
    let _ = sqlx::query("DELETE FROM strategy_preset WHERE name = $1")
        .bind(name)
        .execute(pool)
        .await;

    // 條件不通過回測驗證：hold_days 7。
    let (status, json) = admin("PUT", name, r#"{"query":"from=1993-03-01&hold_days=7"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("hold_days")
    );
    let (status, json) = admin(
        "PUT",
        name,
        r#"{"query":"from=1993-03-01&hold_days=5&rebalance_offset=5"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");

    let (status, json) = admin(
        "PUT",
        name,
        r#"{"query":"?from=1993-03-01&to=1993-03-15&hold_days=5&limit=2","description":" 測試 ","alert":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        json["query"],
        "from=1993-03-01&to=1993-03-15&hold_days=5&limit=2"
    );
    assert_eq!(json["description"], "測試");

    let (status, json) = admin("GET", "", "").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(
        json.as_array()
            .expect("admin presets array")
            .iter()
            .any(|preset| preset["name"] == name && preset["alert"] == true)
    );

    let (status, json) = get("/api/v1/strategy-presets").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let saved = json["presets"]
        .as_array()
        .expect("presets array")
        .iter()
        .find(|preset| preset["name"] == name)
        .cloned()
        .expect("列出剛存的策略");
    assert_eq!(saved["alert"], true);

    // 引用策略：與直接帶條件相同；另帶 limit=1 覆蓋存的 limit=2。
    let encoded = urlencoding::encode(name);
    let (status, json) = get(&format!("/api/v1/stocks/screen/backtest?preset={encoded}")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["preset"], name);
    assert_eq!(json["limit"], 2);
    assert_eq!(json["summary"]["total_return"], 14.48);
    let (_, json) = get(&format!(
        "/api/v1/stocks/screen/backtest?preset={encoded}&limit=1"
    ))
    .await;
    assert_eq!(json["limit"], 1);
    let (status, json) = get(&format!(
        "/api/v1/stocks/screen/backtest/offsets?preset={encoded}"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["hold_days"], 5);

    let (status, json) = get("/api/v1/stocks/screen/backtest?preset=79979%20不存在").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
    assert_eq!(json["error"], "查無策略「79979 不存在」");

    let (status, _) = admin("DELETE", name, "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = admin("DELETE", name, "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    cleanup(pool).await;
}
