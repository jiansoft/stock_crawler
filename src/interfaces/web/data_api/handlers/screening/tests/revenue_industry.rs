//! 條件選股整合測試：月營收創新高與排除產業。

use super::*;

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

/// 營收加速月數的範圍邊界，且單獨即可成立篩選。
#[test]
fn screening_revenue_acceleration_boundaries() {
    for (months, ok) in [(1, true), (11, true), (0, false), (12, false)] {
        let mut params = screening_params();
        params.min_revenue_acceleration_months = Some(months);
        assert_eq!(
            validate_screening_params(&params).is_ok(),
            ok,
            "min_revenue_acceleration_months {months}"
        );
    }
}

/// 以固定查詢日 1988-03-05（最新月營收 1988-01）計算營收加速月數：
/// - RA：1987-10～1988-01 年增 5、10、20、30%，連續 3 個月提高。
/// - RB：10、30、20、25%：1 月高於 12 月，但 12 月低於 11 月，只有 1 個月。
/// - RC：1987-11 為 10%、1988-01 為 30%，中間缺 12 月，月份不相連為 0。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_counts_revenue_acceleration_months() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_counts_revenue_acceleration_months：無資料庫連接");
        return;
    }
    const SYMBOLS: [&str; 3] = ["79979RA", "79979RB", "79979RC"];
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
    let months: [(&str, &[(i64, i32)]); 3] = [
        (
            "79979RA",
            &[(198710, 5), (198711, 10), (198712, 20), (198801, 30)],
        ),
        (
            "79979RB",
            &[(198710, 10), (198711, 30), (198712, 20), (198801, 25)],
        ),
        ("79979RC", &[(198711, 10), (198801, 30)]),
    ];
    for (symbol, rows) in months {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '營收加速', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        for (month, yoy) in rows {
            sqlx::query(
                r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "ComparedWithLastYearSameMonth")
                   VALUES ($1, $1, $2, 979790, $3)"#,
            )
            .bind(symbol)
            .bind(month)
            .bind(yoy)
            .execute(pool)
            .await
            .expect("種入月營收");
        }
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let run = |min_months: Option<u8>| async move {
        let mut params = screening_params();
        params.min_monthly_revenue = Some(979_790.0);
        params.min_revenue_acceleration_months = min_months;
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
        let mut rows: Vec<(String, serde_json::Value)> = json["stocks"]
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
                    stock["revenue_acceleration_months"].clone(),
                )
            })
            .collect();
        rows.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        rows
    };
    let all = run(None).await;
    let one = run(Some(1)).await;
    let three = run(Some(3)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            ("79979RA".to_owned(), serde_json::json!(3)),
            ("79979RB".to_owned(), serde_json::json!(1)),
            ("79979RC".to_owned(), serde_json::json!(0)),
        ]
    );
    let symbols = |rows: &[(String, serde_json::Value)]| {
        rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>()
    };
    assert_eq!(symbols(&one), ["79979RA", "79979RB"]);
    assert_eq!(symbols(&three), ["79979RA"]);
}
