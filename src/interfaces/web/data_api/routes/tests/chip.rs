//! 籌碼 endpoint 的驗證、參數與資料庫語意測試。

use super::*;

/// 籌碼 endpoint：未帶金鑰 401、參數不合法 422、未知代號 404，
/// 種入一檔假代號後回傳每日籌碼（新到舊）、連續天數、千張大戶、董監合計與主力進出。
#[tokio::test]
async fn chip_endpoint_rejects_missing_bearer_key() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/stocks/2330/chip")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn chip_endpoint_db_semantics() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 chip_endpoint_db_semantics：無資料庫連接");
        return;
    }
    const SYMBOL: &str = "79979CP";
    let cleanup = || async {
        for sql in [
            "DELETE FROM chip_daily WHERE stock_symbol = $1",
            "DELETE FROM holder_distribution WHERE stock_symbol = $1",
            "DELETE FROM insider_holding WHERE stock_symbol = $1",
            "DELETE FROM broker_flow WHERE stock_symbol = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
    };
    cleanup().await;
    for sql in [
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '籌碼測試', $1, 2, false)"#,
        r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net, margin_previous, margin_balance, short_previous, short_balance)
           VALUES ('2026-04-28', $1, -10, 5, 0, 100, 110, 3, 2), ('2026-04-29', $1, 30, 5, 1, 110, 120, 2, 2), ('2026-04-30', $1, 20, -5, 1, 120, 115, 2, 4)"#,
        r#"INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders) VALUES ('2026-04-24', $1, 50, 70.5, 3000)"#,
        r#"INSERT INTO insider_holding ("month", stock_symbol, title, name, shares, pledged) VALUES ('2026-03-01', $1, '董事', '甲', 1000, 0), ('2026-04-01', $1, '董事', '甲', 1000, 250), ('2026-04-01', $1, '監察人', '乙', 1000, 0)"#,
        r#"INSERT INTO broker_flow ("date", stock_symbol, buy_total, sell_total, main_share, buyers, sellers) VALUES ('2026-04-30', $1, 120, 80, 3.5, '[{"name":"凱基-台北","buy":150,"sell":30,"net":120,"share":"6.98"}]', '[]')"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("種入籌碼測試資料");
    }

    let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "chip-integration-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    });
    let get = |path: String| {
        let key = key.clone();
        async move {
            let response = router()
                .oneshot(
                    Request::get(&path)
                        .header("Authorization", format!("Bearer {key}"))
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body should be readable");
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };

    for path in [
        format!("/api/v1/stocks/{SYMBOL}/chip?days=0"),
        format!("/api/v1/stocks/{SYMBOL}/chip?days=121"),
    ] {
        assert_eq!(
            get(path.clone()).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}"
        );
    }
    assert_eq!(
        get("/api/v1/stocks/79979ZZ/chip".to_owned()).await.0,
        StatusCode::NOT_FOUND
    );

    let (status, json) = get(format!("/api/v1/stocks/{SYMBOL}/chip?days=2")).await;
    cleanup().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2026-04-30");
    let daily = json["daily"].as_array().expect("daily array");
    assert_eq!(daily.len(), 2);
    assert_eq!(daily[0]["date"], "2026-04-30");
    assert_eq!(daily[0]["total_net"], 16);
    assert_eq!(daily[0]["margin_change"], -5);
    assert_eq!(json["streak"]["foreign_days"], 2);
    assert_eq!(json["streak"]["trust_days"], -1);
    assert_eq!(json["holder_distribution"][0]["major_percent"], 70.5);
    assert_eq!(json["insider"]["month"], "2026-04");
    assert_eq!(json["insider"]["insiders"], 2);
    assert_eq!(json["insider"]["pledge_percent"], 12.5);
    assert_eq!(json["broker_flow"]["main_net"], 40);
    assert_eq!(json["broker_flow"]["buyers"][0]["share"], 6.98);
}
