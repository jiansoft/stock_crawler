//! 以真實 PostgreSQL 執行條件選股與驗證執行計畫的整合測試；沒有資料庫時跳過。

use super::*;

/// P0-4：以實際 PostgreSQL 對 Phase 2/3 核心查詢執行 EXPLAIN ANALYZE。
///
/// 測試輸出文字 plan 供人工記錄 latency 與 scan 類型，並固定條件選股
/// 四個「每股最新一期」子查詢使用既有索引；沒有資料庫時明確跳過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn phase2_phase3_query_plans_use_existing_indexes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 P0-4 EXPLAIN：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    // EXPLAIN 只需要型別正確的查詢參數，不需要依賴資料庫預先匯入股票。
    // 使用固定代號可讓全新 CI 資料庫也能驗證執行計畫，且不會寫入測試資料。
    let symbol = "2330";

    // SQL 與 production 使用相同 WHERE、JOIN、排序及 LIMIT；EXPLAIN 不寫資料。
    let valuation: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT security_code, date, closing_price FROM estimate WHERE security_code = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30)) ORDER BY date DESC LIMIT 1"#)
        .bind(symbol).bind(Option::<NaiveDate>::None).fetch_all(pool).await.expect("valuation EXPLAIN");
    let breadth: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) WITH endpoint AS (SELECT MAX(date) AS date FROM daily_stock_price_stats WHERE stock_exchange_market_id = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30))) SELECT s.date FROM daily_stock_price_stats s CROSS JOIN endpoint e WHERE s.stock_exchange_market_id = $1 AND s.date <= e.date ORDER BY s.date DESC LIMIT $3"#)
        .bind(0_i32).bind(Option::<NaiveDate>::None).bind(20_i64).fetch_all(pool).await.expect("breadth EXPLAIN");
    let ranking: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT y.security_code FROM yield_rank y JOIN stocks s ON s.stock_symbol = y.security_code JOIN "DailyQuotes" q ON q."Serial" = y.daily_quotes_serial JOIN dividend d ON d.serial = y.dividend_serial WHERE y.date = (SELECT MAX(date) FROM yield_rank) AND s.stock_exchange_market_id IN (2,4) ORDER BY y.yield DESC, y.security_code ASC LIMIT 20"#)
        .fetch_all(pool).await.expect("ranking EXPLAIN");
    let screen_sql = format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {SCREEN_STOCKS_SQL}\nORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC\nLIMIT $24"
    );
    let screen: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(screen_sql.as_str()))
        .bind(0_i32)
        .bind(Option::<i32>::None)
        .bind(chrono::Local::now().date_naive())
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<&str>::None)
        .bind(Option::<i16>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(false)
        .bind(Option::<i16>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Vec::<String>::new())
        .bind(Vec::<i16>::new())
        .bind(Option::<i16>::None)
        .bind(Vec::<String>::new())
        .bind(Vec::<i16>::new())
        .bind(Vec::<i16>::new())
        .bind(Option::<i16>::None)
        .bind(Option::<i16>::None)
        .bind(20_i64)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(false)
        .bind(false)
        .bind(Option::<Vec<i32>>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .fetch_all(pool)
        .await
        .expect("screen EXPLAIN");

    for (name, plan) in [
        ("valuation", valuation),
        ("breadth", breadth),
        ("yield-ranking", ranking),
        ("screen", screen),
    ] {
        assert!(!plan.is_empty(), "{name} 應回傳 plan");
        println!("\n===== {name} =====\n{}", plan.join("\n"));
        if name == "screen" {
            assert!(
                plan.join("\n")
                    .contains("yield_rank-security_code-date-desc-idx"),
                "screen 每股最新殖利率必須走複合索引"
            );
        }
    }
}

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

/// 營收創歷史新高：以固定查詢日 1988-03-05 執行。SR1 1988-01 營收 200 高於之前 12 個月（100），
/// SR2 1988-01 營收 90 不是新高，SR3 之前只有 5 個月不判斷（`null`），SR4 創新高的月份是 1987-10，
/// 距查詢日超過 3 個月，過期。只篩選創新高時只剩 SR1。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_at_filters_revenue_record() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_revenue_record：無資料庫連接");
        return;
    }
    const SYMBOLS: [&str; 4] = ["79979SR1", "79979SR2", "79979SR3", "79979SR4"];
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "Revenue" WHERE "SecurityCode" = ANY($1)"#,
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(&SYMBOLS[..]).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '營收新高', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    // 之前的月份每月營收 100；最後一個月 SR1 200、SR2 90、SR3 200（只有 5 個月歷史）、SR4 200（1987-10）。
    for (symbol, first, last, latest) in [
        ("79979SR1", "1987-01-01", "1987-12-01", "1988-01-01"),
        ("79979SR2", "1987-01-01", "1987-12-01", "1988-01-01"),
        ("79979SR3", "1987-08-01", "1987-12-01", "1988-01-01"),
        ("79979SR4", "1986-10-01", "1987-09-01", "1987-10-01"),
    ] {
        let monthly = match symbol {
            "79979SR2" => 90,
            _ => 200,
        };
        sqlx::query(
            r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
               SELECT $1, $1, to_char(month, 'YYYYMM')::bigint, 100
               FROM generate_series($2::date, $3::date, INTERVAL '1 month') AS month
               UNION ALL
               SELECT $1, $1, to_char($4::date, 'YYYYMM')::bigint, $5"#,
        )
        .bind(symbol)
        .bind(first)
        .bind(last)
        .bind(latest)
        .bind(monthly)
        .execute(pool)
        .await
        .expect("種入月營收");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |record: Option<bool>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_monthly_revenue = Some(0.0);
        params.revenue_record = record;
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
        let mut stocks: Vec<(String, serde_json::Value)> = json["stocks"]
            .as_array()
            .expect("stocks")
            .iter()
            .filter(|stock| {
                stock["stock_symbol"]
                    .as_str()
                    .is_some_and(|symbol| SYMBOLS.contains(&symbol))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    stock["revenue_record"].clone(),
                )
            })
            .collect();
        stocks.sort_by(|a, b| a.0.cmp(&b.0));
        stocks
    };

    // 月營收門檻 0：過期的 SR4 沒有營收，不列。
    assert_eq!(
        run(None).await,
        [
            ("79979SR1".to_owned(), serde_json::json!(true)),
            ("79979SR2".to_owned(), serde_json::json!(false)),
            ("79979SR3".to_owned(), serde_json::Value::Null),
        ]
    );
    assert_eq!(
        run(Some(true)).await,
        [("79979SR1".to_owned(), serde_json::json!(true))]
    );

    cleanup().await;
}

/// 排除產業：以固定查詢日 1988-03-05 執行，金融保險業（17）的 SX1 被排除，SX2（產業 1）與 SX3（產業 2）
/// 保留；排除兩個產業時只剩 SX3。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_at_excludes_industries() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_excludes_industries：無資料庫連接");
        return;
    }
    const SYMBOLS: [&str; 3] = ["79979SX1", "79979SX2", "79979SX3"];
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "Revenue" WHERE "SecurityCode" = ANY($1)"#,
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(&SYMBOLS[..]).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, industry) in [("79979SX1", 17), ("79979SX2", 1), ("79979SX3", 2)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '排除產業', $1, 2, $2)"#,
        )
        .bind(symbol)
        .bind(industry)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "ComparedWithLastYearSameMonth")
               VALUES ($1, $1, 198801, 979790, 50)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入月營收");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |excluded: &'static str| async move {
        let mut params = screening_params();
        params.min_monthly_revenue = Some(979_790.0);
        params.exclude_industry_ids = Some(excluded.to_owned());
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
        let mut symbols: Vec<String> = json["stocks"]
            .as_array()
            .expect("stocks")
            .iter()
            .filter_map(|stock| stock["stock_symbol"].as_str())
            .filter(|symbol| SYMBOLS.contains(symbol))
            .map(str::to_owned)
            .collect();
        symbols.sort_unstable();
        symbols
    };
    assert_eq!(run("17").await, ["79979SX2", "79979SX3"]);
    assert_eq!(run("17,1").await, ["79979SX3"]);

    cleanup().await;
}
