//! 條件選股整合測試：本益比／股價淨值比上限與三率門檻。

use super::*;

/// 本益比與股價淨值比：取查詢日 1988-03-05 之前 10 天內最新一筆日線，0 轉成 `null` 且不符合上限；
/// 超過 10 天的日線不採用。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_valuation_caps() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_valuation_caps：無資料庫連接");
        return;
    }
    // PA：03-03 本益比 12、淨值比 1.5（02-01 的 5 倍已被取代）；PB：本益比 0（虧損）、淨值比 0.9；
    // PC：只有 02-01 的日線，超過 10 天。
    const SYMBOLS: [&str; 3] = ["79979PA", "79979PB", "79979PC"];
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOLS).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '估值選股', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "PriceEarningRatio", "price-to-book_ratio") VALUES
           ('1988-02-01', '79979PA', 5, 0.5), ('1988-03-03', '79979PA', 12, 1.5),
           ('1988-03-03', '79979PB', 0, 0.9), ('1988-02-01', '79979PC', 3, 0.3)"#,
    )
    .execute(pool)
    .await
    .expect("種入日線本益比");

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |max_pe: Option<f64>, max_pb: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.max_pe = max_pe;
        params.max_pb = max_pb;
        params.sort_by = Some("pe_ratio".to_owned());
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .filter(|stock| {
                stock["stock_symbol"]
                    .as_str()
                    .is_some_and(|symbol| SYMBOLS.contains(&symbol))
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    let pe_capped = run(Some(12.0), None).await;
    let pe_tight = run(Some(11.9), None).await;
    let pb_capped = run(None, Some(1.0)).await;
    cleanup().await;
    assert_eq!(pe_capped.len(), 1, "{pe_capped:?}");
    assert_eq!(pe_capped[0]["stock_symbol"], "79979PA");
    assert_eq!(pe_capped[0]["pe_ratio"], 12.0);
    assert_eq!(pe_capped[0]["pb_ratio"], 1.5);
    assert_eq!(pe_capped[0]["ratio_date"], "1988-03-03");
    assert!(pe_tight.is_empty(), "{pe_tight:?}");
    assert_eq!(pb_capped.len(), 1, "{pb_capped:?}");
    assert_eq!(pb_capped[0]["stock_symbol"], "79979PB");
    assert!(pb_capped[0]["pe_ratio"].is_null(), "{pb_capped:?}");
    assert_eq!(pb_capped[0]["pb_ratio"], 0.9);
}

/// 三率三升與營業利益率：以固定查詢日 1988-03-05 執行，最新一季須在兩季內；
/// 去年同季沒有資料時三率三升為 `null`，過期的一季只保留期間。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_margins() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_margins：無資料庫連接");
        return;
    }
    const SYMBOLS: [&str; 4] = ["79979SM1", "79979SM2", "79979SM3", "79979SM4"];
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM income_statement WHERE stock_symbol = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(&SYMBOLS[..]).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '三率選股', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    // 營收 100，毛利／營業利益／稅後淨利：SM1 1987 Q4 40／20／15、去年同季 30／10／5（三率三升）；
    // SM2 1987 Q4 50／30／20、去年同季 55／25／15（毛利率下降）；SM3 只有 1987 Q4 30／10／8；
    // SM4 最新是 1987 Q2（落後三季，過期）。
    sqlx::query(
        "INSERT INTO income_statement
             (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, operating_profit, net_income)
         VALUES ('79979SM1', 1987, 'single', 'Q4', 'yahoo', 100, 40, 20, 15),
                ('79979SM1', 1986, 'single', 'Q4', 'yahoo', 100, 30, 10, 5),
                ('79979SM2', 1987, 'single', 'Q4', 'yahoo', 100, 50, 30, 20),
                ('79979SM2', 1986, 'single', 'Q4', 'yahoo', 100, 55, 25, 15),
                ('79979SM3', 1987, 'single', 'Q4', 'yahoo', 100, 30, 10, 8),
                ('79979SM4', 1987, 'single', 'Q2', 'yahoo', 100, 60, 40, 30)",
    )
    .execute(pool)
    .await
    .expect("種入損益表");

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |operating: Option<f64>, up: Option<bool>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_operating_margin_percent = operating;
        params.margins_yoy_up = up;
        params.sort_by = Some("operating_margin".to_owned());
        params.sort_order = Some("desc".to_owned());
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .filter(|stock| {
                SYMBOLS
                    .iter()
                    .any(|symbol| stock["stock_symbol"] == *symbol)
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    let up = run(None, Some(true)).await;
    let operating = run(Some(25.0), None).await;
    let all = run(None, None).await;
    let positive = run(Some(5.0), None).await;
    cleanup().await;

    assert_eq!(up.len(), 1, "{up:?}");
    assert_eq!(up[0]["stock_symbol"], "79979SM1");
    assert_eq!(up[0]["operating_margin_percent"], 20.0);
    assert_eq!(up[0]["net_margin_percent"], 15.0);
    assert_eq!(up[0]["margins_yoy_up"], true);
    assert_eq!(up[0]["margin_period"], "1987-Q4");
    assert_eq!(operating.len(), 1, "{operating:?}");
    assert_eq!(operating[0]["stock_symbol"], "79979SM2");
    assert_eq!(operating[0]["margins_yoy_up"], false);
    // 依營業利益率降冪：SM2 30%、SM1 20%、SM3 10%，過期的 SM4 為 null 排最後。
    let order: Vec<&str> = all
        .iter()
        .filter_map(|stock| stock["stock_symbol"].as_str())
        .collect();
    assert_eq!(order, ["79979SM2", "79979SM1", "79979SM3", "79979SM4"]);
    assert!(all[2]["margins_yoy_up"].is_null());
    assert!(all[3]["operating_margin_percent"].is_null());
    assert_eq!(all[3]["margin_period"], "1987-Q2");
    assert_eq!(positive.len(), 3, "{positive:?}");
}
