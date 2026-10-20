//! 條件選股整合測試：法人連續買超天數與平均成交金額門檻。

use super::*;

/// 法人連續買超門檻：以固定歷史查詢日 1988-03-05 執行選股，資料日回推到 03-03；
/// 外資連買 3 天、投信連買 2 天的股票在門檻以內出現並輸出天數，超過門檻即不符合。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_chip_streaks() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_chip_streaks：無資料庫連接");
        return;
    }
    const SYMBOL: &str = "79979SC";
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM chip_daily WHERE stock_symbol = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
    };
    cleanup().await;
    for sql in [
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, '籌碼選股', $1, 2, 1)"#,
        r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net)
           VALUES ('1988-03-01', $1, 1000, -1, 0), ('1988-03-02', $1, 1000, 1, 0), ('1988-03-03', $1, 1000, 1, 0)"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("種入選股籌碼資料");
    }
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |foreign: Option<u8>, trust: Option<u8>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_foreign_streak = foreign;
        params.min_trust_streak = trust;
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        serde_json::from_slice::<serde_json::Value>(&bytes).expect("JSON 回應")
    };

    let json = run(Some(3), Some(2)).await;
    let excluded = run(None, Some(3)).await;
    cleanup().await;
    assert_eq!(json["chip_as_of"], "1988-03-03");
    let stock = json["stocks"]
        .as_array()
        .expect("stocks 陣列")
        .iter()
        .find(|stock| stock["stock_symbol"] == SYMBOL)
        .unwrap_or_else(|| panic!("連續天數符合，應出現：{json}"));
    assert_eq!(stock["foreign_streak"], 3);
    assert_eq!(stock["trust_streak"], 2);
    assert!(
        excluded["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .all(|stock| stock["stock_symbol"] != SYMBOL),
        "投信只連買 2 天，不應出現：{excluded}"
    );
}

/// 平均成交金額門檻：以固定歷史查詢日 1988-03-05 執行選股；近 20 筆日線平均 6,000 萬元的股票
/// 在門檻 60 百萬元以內出現並輸出平均值，門檻 61 即不符合；45 天以前的日線不算。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_avg_trade_value() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_avg_trade_value：無資料庫連接");
        return;
    }
    const SYMBOL: &str = "79979SV";
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
    };
    cleanup().await;
    for sql in [
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, '流動性選股', $1, 2, 1)"#,
        // 01-04 的 10 億元在 45 天之外，不列入平均；03-02、03-03 平均 6,000 萬元。
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "TradeValue")
           VALUES ('1988-01-04', $1, 1000000000), ('1988-03-02', $1, 40000000), ('1988-03-03', $1, 80000000)"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("種入選股日線資料");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |minimum: u32| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_avg_trade_value = Some(minimum);
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        serde_json::from_slice::<serde_json::Value>(&bytes).expect("JSON 回應")
    };

    let json = run(60).await;
    let excluded = run(61).await;
    cleanup().await;
    let stock = json["stocks"]
        .as_array()
        .expect("stocks 陣列")
        .iter()
        .find(|stock| stock["stock_symbol"] == SYMBOL)
        .unwrap_or_else(|| panic!("平均成交金額符合，應出現：{json}"));
    assert_eq!(stock["avg_trade_value_20d"], 60.0);
    assert!(
        excluded["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .all(|stock| stock["stock_symbol"] != SYMBOL),
        "平均 6,000 萬元低於 61 百萬元，不應出現：{excluded}"
    );
}
