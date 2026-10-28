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

/// 在明確啟用的隔離 PostgreSQL 驗證明細與完整連續天數的獨立性。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn chip_endpoint_db_semantics() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    sqlx::query("SELECT 1")
        .execute(pool)
        .await
        .expect("整合測試必須連上明確設定的隔離 PostgreSQL");
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
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;

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

    // 切成一日明細後，完整外資連買仍為兩日，不應跟著被截成一日。
    let (one_status, one_day) = get(format!("/api/v1/stocks/{SYMBOL}/chip?days=1")).await;
    let (status, json) = get(format!("/api/v1/stocks/{SYMBOL}/chip?days=2")).await;
    cleanup().await;
    assert_eq!(one_status, StatusCode::OK);
    assert_eq!(one_day["daily"].as_array().expect("daily array").len(), 1);
    assert_eq!(one_day["streak"]["foreign_days"], 2);
    assert_eq!(one_day["streak"], json["streak"]);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2026-04-30");
    let daily = json["daily"].as_array().expect("daily array");
    assert_eq!(daily.len(), 2);
    assert_eq!(daily[0]["date"], "2026-04-30");
    assert_eq!(daily[0]["total_net"], 16);
    assert_eq!(daily[0]["margin_change"], -5);
    assert_eq!(daily[0]["main_net"], 40);
    assert_eq!(daily[0]["main_share"], 3.5);
    assert!(daily[1]["main_net"].is_null(), "沒有主力進出的日子為 null");
    assert_eq!(json["streak"]["foreign_days"], 2);
    assert_eq!(json["streak"]["trust_days"], -1);
    assert_eq!(json["holder_distribution"][0]["major_percent"], 70.5);
    assert_eq!(json["insider"]["month"], "2026-04");
    assert_eq!(json["insider"]["insiders"], 2);
    assert_eq!(json["insider"]["pledge_percent"], 12.5);
    assert_eq!(json["broker_flow"]["main_net"], 40);
    assert_eq!(json["broker_flow"]["buyers"][0]["share"], 6.98);
}

/// 真實 DB 的完整連續天數須大於五日視窗，且不受明細 days 截短。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn chip_full_streak_survives_five_day_detail_window() {
    use super::support::{get, pool_or_skip};
    let pool = pool_or_skip("chip_full_streak_survives_five_day_detail_window")
        .await
        .expect("隔離 DB 必須可用");
    let symbol = "79979CS";
    for sql in [
        "DELETE FROM chip_daily WHERE stock_symbol=$1",
        "DELETE FROM stocks WHERE stock_symbol=$1",
    ] {
        sqlx::query(sql).bind(symbol).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id) VALUES ($1,'連續天數測試',$1,2)").bind(symbol).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO chip_daily(date,stock_symbol,foreign_net,trust_net,dealer_net) SELECT d::date,$1,100,-100,0 FROM generate_series('1991-01-01'::date,'1991-02-28'::date,'1 day') d WHERE EXTRACT(ISODOW FROM d)<=5 ORDER BY d LIMIT 20").bind(symbol).execute(pool).await.unwrap();
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;
    let (status, short) = get(&format!("/api/v1/stocks/{symbol}/chip?days=5")).await;
    assert_eq!(status, StatusCode::OK, "{short}");
    let (status, long) = get(&format!("/api/v1/stocks/{symbol}/chip?days=60")).await;
    assert_eq!(status, StatusCode::OK, "{long}");
    assert_eq!(short["daily"].as_array().unwrap().len(), 5);
    assert_eq!(long["daily"].as_array().unwrap().len(), 20);
    assert_eq!(short["streak"], long["streak"]);
    assert_eq!(short["streak"]["foreign_days"], 20);
    assert_eq!(short["streak"]["trust_days"], -20);
    for sql in [
        "DELETE FROM chip_daily WHERE stock_symbol=$1",
        "DELETE FROM stocks WHERE stock_symbol=$1",
    ] {
        sqlx::query(sql).bind(symbol).execute(pool).await.unwrap();
    }
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;
}

/// 持股分級 endpoint 沒帶金鑰時回 401。
#[tokio::test]
async fn holder_levels_endpoint_rejects_missing_bearer_key() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/stocks/2330/holder-levels")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// 持股分級：預設最新一週、`date` 取該日以前最近一週、沒有資料 404；籌碼的週摘要帶 400 張欄位（缺值為 null）。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn holder_levels_and_chip_400_columns() {
    use super::support::{get, pool_or_skip};
    let pool = pool_or_skip("holder_levels_and_chip_400_columns")
        .await
        .expect("隔離 DB 必須可用");
    let symbol = "79979HL";
    let cleanup = || async {
        for sql in [
            "DELETE FROM holder_distribution_level WHERE stock_symbol=$1",
            "DELETE FROM holder_distribution WHERE stock_symbol=$1",
            "DELETE FROM stocks WHERE stock_symbol=$1",
        ] {
            sqlx::query(sql).bind(symbol).execute(pool).await.unwrap();
        }
    };
    cleanup().await;
    for sql in [
        "INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id) VALUES ($1,'分級測試',$1,2)",
        r#"INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders, holders_400, shares_400, percent_400)
           VALUES ('2019-03-15', $1, 50, 70.5, 3000, NULL, NULL, NULL), ('2019-03-22', $1, 51, 70.75, 3010, 80, 900000, 75.5)"#,
        r#"INSERT INTO holder_distribution_level ("date", stock_symbol, level, holders, shares, percent)
           VALUES ('2019-03-15', $1, 1, 2000, 500000, 5.0), ('2019-03-15', $1, 15, 50, 7050000, 70.5), ('2019-03-15', $1, 17, 3000, 10000000, 100),
                  ('2019-03-22', $1, 1, 2010, 490000, 4.9), ('2019-03-22', $1, 12, 29, 150000, 1.5), ('2019-03-22', $1, 15, 51, 7075000, 70.75),
                  ('2019-03-22', $1, 16, 1, 0, 0), ('2019-03-22', $1, 17, 3010, 10000000, 100)"#,
    ] {
        sqlx::query(sql).bind(symbol).execute(pool).await.unwrap();
    }

    let (bad_status, _) = get(&format!(
        "/api/v1/stocks/{symbol}/holder-levels?date=2019-3"
    ))
    .await;
    let (unknown_status, _) = get("/api/v1/stocks/79979ZZ/holder-levels").await;
    let (latest_status, latest) = get(&format!("/api/v1/stocks/{symbol}/holder-levels")).await;
    let (earlier_status, earlier) = get(&format!(
        "/api/v1/stocks/{symbol}/holder-levels?date=2019-03-21"
    ))
    .await;
    let (none_status, none) = get(&format!(
        "/api/v1/stocks/{symbol}/holder-levels?date=2019-03-01"
    ))
    .await;
    let (chip_status, chip) = get(&format!("/api/v1/stocks/{symbol}/chip?days=1")).await;
    cleanup().await;

    assert_eq!(bad_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);

    assert_eq!(latest_status, StatusCode::OK, "{latest}");
    assert_eq!(latest["date"], "2019-03-22");
    let levels = latest["levels"].as_array().expect("levels array");
    assert_eq!(
        levels
            .iter()
            .map(|item| item["level"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 12, 15]
    );
    assert_eq!(levels[1]["label"], "400,001～600,000 股");
    assert_eq!(levels[2]["percent"], 70.75);
    assert_eq!(latest["adjustment"]["holders"], 1);
    assert_eq!(latest["total"]["holders"], 3010);

    assert_eq!(earlier_status, StatusCode::OK, "{earlier}");
    assert_eq!(earlier["date"], "2019-03-15");
    assert!(earlier["adjustment"].is_null());
    assert_eq!(earlier["total"]["holders"], 3000);

    assert_eq!(none_status, StatusCode::NOT_FOUND, "{none}");

    assert_eq!(chip_status, StatusCode::OK, "{chip}");
    let weeks = chip["holder_distribution"].as_array().expect("weeks array");
    assert_eq!(weeks[0]["holders_400"], 80);
    assert_eq!(weeks[0]["percent_400"], 75.5);
    assert!(weeks[1]["holders_400"].is_null());
    assert!(weeks[1]["percent_400"].is_null());
}
