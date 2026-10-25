//! 再平衡端點的隔離資料庫語意：三維度、設定覆蓋、缺價與有限值驗證。
use super::support::{get, pool_or_skip};
use super::*;
use serde_json::{Value, json};

/// 帶同一組測試 Bearer 金鑰呼叫目標配置的 PUT。
async fn put(body: Value) -> (StatusCode, Value) {
    let key = std::env::var("DATA_API_KEY").expect("先以 get 建立測試金鑰");
    let response = router()
        .oneshot(
            Request::put("/api/v1/portfolio/targets")
                .header("Authorization", format!("Bearer {key}"))
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

/// 實際 API 與 repository 流程均須通過，OpenAPI 結構檢查不能取代本測試。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn rebalance_targets_and_exposure_db_semantics() {
    let pool = pool_or_skip("rebalance_targets_and_exposure_db_semantics")
        .await
        .expect("隔離 DB 必須可用");
    for sql in [
        "DELETE FROM portfolio_target WHERE member_id='79979'",
        "DELETE FROM portfolio_target_dimension WHERE member_id='79979'",
        "DELETE FROM stock_ownership_details WHERE member_id=79979",
        "DELETE FROM etf_holding WHERE stock_symbol='79979EF'",
        "DELETE FROM etf_snapshot WHERE stock_symbol='79979EF'",
        "DELETE FROM last_daily_quotes WHERE stock_symbol IN ('79979RAA','79979EF')",
        "DELETE FROM stocks WHERE stock_symbol IN ('79979RAA','79979EF')",
    ] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    let (status, _) = get("/api/v1/portfolio/rebalance?member_id=79979").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for query in [
        "member_id=-1",
        "dimension=bad",
        "lot_size=2",
        "tolerance_percent=-1",
        "tolerance_percent=NaN",
        "available_cash=NaN",
        "cash=-1",
        "fee_discount=2",
    ] {
        let (status, body) = get(&format!("/api/v1/portfolio/rebalance?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
    }
    for body in [
        json!({"member_id":-1,"dimension":"stock","targets":[]}),
        json!({"dimension":"bad","targets":[]}),
        json!({"dimension":"stock","targets":[{"target_key":"A","target_percent":33.335}]}),
        json!({"dimension":"stock","targets":[{"target_key":"A","target_percent":60},{"target_key":"B","target_percent":60}]}),
        json!({"dimension":"stock","targets":[{"target_key":"A","target_percent":-1}]}),
    ] {
        assert_eq!(put(body).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    sqlx::query("INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id,stock_industry_id) VALUES ('79979RAA','測試股','79979RAA',2,24),('79979EF','測試ETF','79979EF',2,9001)").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO last_daily_quotes(stock_symbol,date,closing_price) VALUES ('79979RAA','2026-10-23',100),('79979EF','2026-10-23',100)").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO stock_ownership_details(member_id,security_code,share_quantity,is_sold) VALUES (79979,'79979RAA',1000,false),(79979,'79979EF',1000,false)").execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO etf_snapshot(stock_symbol,name,market_id) VALUES ('79979EF','測試ETF',2)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO etf_holding(stock_symbol,rank,holding_symbol,holding_name,weight,as_of) VALUES ('79979EF',1,'79979RAA','測試股',60,'2026-10-23')").execute(pool).await.unwrap();
    for (dimension, key) in [
        ("stock", "79979RAA"),
        ("industry", "半導體業"),
        ("etf_look_through", "79979RAA"),
    ] {
        let (status,saved)=put(json!({"member_id":79979,"dimension":dimension,"targets":[{"target_key":key,"target_percent":60}]})).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["target_cash_percent"], 40.0);
        let (status, result) = get(&format!(
            "/api/v1/portfolio/rebalance?member_id=79979&dimension={dimension}&lot_size=1&cash=0"
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["total_equity_value"], 200000.0);
        assert_eq!(result["etf_coverage"][0]["covered_weight_percent"], 60.0);
        assert_eq!(result["uncovered_value"], 40000.0);
        assert_eq!(result["etf_coverage"][0]["as_of"], "2026-10-23");
        assert!(!result["warnings"].as_array().unwrap().is_empty());
        if dimension == "industry" {
            assert_eq!(result["industry_items"][0]["current_value"], 160000.0);
        }
        if dimension == "etf_look_through" {
            assert_eq!(result["etf_items"][0]["total_value"], 160000.0);
        }
        if dimension == "stock" {
            assert!(
                result["stock_items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|i| i["action"] == "buy")
            );
        }
    }
    let (_, all) = get("/api/v1/portfolio/targets?member_id=79979").await;
    assert!(all["target_cash_percent"].is_null());
    for dimension in ["stock", "industry", "etf_look_through"] {
        assert_eq!(all["cash_percent_by_dimension"][dimension], 40.0);
    }
    assert!(
        all["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["dimension"].is_string())
    );
    assert_eq!(
        get("/api/v1/portfolio/targets?member_id=-1").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get("/api/v1/portfolio/targets?dimension=bad").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get("/api/v1/portfolio/targets?member_id=79979&dimension=stock")
            .await
            .1["target_cash_percent"],
        40.0
    );
    let (status, _) = put(json!({"member_id":79979,"dimension":"stock","targets":[]})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, cash) = get("/api/v1/portfolio/rebalance?member_id=79979&lot_size=1").await;
    assert_eq!(status, StatusCode::OK, "{cash}");
    assert_eq!(cash["target_cash_percent"], 100.0);
    assert!(
        cash["stock_items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["action"] == "sell")
    );
    let etf = cash["stock_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["symbol"] == "79979EF")
        .unwrap();
    assert_eq!(etf["estimated_cost"], 242.5);
    sqlx::query("UPDATE last_daily_quotes SET closing_price=0 WHERE stock_symbol='79979RAA'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        get("/api/v1/portfolio/rebalance?member_id=79979").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    sqlx::query("UPDATE last_daily_quotes SET closing_price=100 WHERE stock_symbol='79979RAA'")
        .execute(pool)
        .await
        .unwrap();
    let (_,saved)=put(json!({"member_id":79979,"dimension":"stock","targets":[{"target_key":"79979UNKNOWN","target_percent":100}]})).await;
    assert_eq!(saved["target_cash_percent"], 0.0);
    assert_eq!(
        get("/api/v1/portfolio/rebalance?member_id=79979").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for sql in [
        "DELETE FROM portfolio_target WHERE member_id='79979'",
        "DELETE FROM portfolio_target_dimension WHERE member_id='79979'",
        "DELETE FROM stock_ownership_details WHERE member_id=79979",
        "DELETE FROM etf_holding WHERE stock_symbol='79979EF'",
        "DELETE FROM etf_snapshot WHERE stock_symbol='79979EF'",
        "DELETE FROM last_daily_quotes WHERE stock_symbol IN ('79979RAA','79979EF')",
        "DELETE FROM stocks WHERE stock_symbol IN ('79979RAA','79979EF')",
    ] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
}
