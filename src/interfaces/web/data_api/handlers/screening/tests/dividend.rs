//! 條件選股：存股條件（近 5 年填息率、平均填息天數、連續配息年數、股利成長年數）。

use serde_json::json;

use super::*;

/// 三個條件的範圍邊界，且單獨即可成立篩選。
#[test]
fn screening_dividend_fill_boundaries() {
    for (value, ok) in [(0.0, true), (100.0, true), (-0.5, false), (100.5, false)] {
        let mut params = screening_params();
        params.min_fill_rate_percent = Some(value);
        assert_eq!(
            validate_screening_params(&params).is_ok(),
            ok,
            "min_fill_rate_percent {value}"
        );
    }
    for (value, ok) in [(1.0, true), (120.0, true), (0.5, false), (121.0, false)] {
        let mut params = screening_params();
        params.max_avg_fill_days = Some(value);
        assert_eq!(
            validate_screening_params(&params).is_ok(),
            ok,
            "max_avg_fill_days {value}"
        );
    }
    for (years, ok) in [(1, true), (30, true), (0, false), (31, false)] {
        let mut params = screening_params();
        params.min_dividend_years = Some(years);
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "min_dividend_years {years}");
        if let Ok(validated) = validated {
            assert_eq!(validated.min_dividend_years, Some(i32::from(years)));
        }
    }
}

/// 股利成長年數的範圍邊界，且單獨即可成立篩選。
#[test]
fn screening_dividend_growth_boundaries() {
    for (years, ok) in [(1, true), (30, true), (0, false), (31, false)] {
        let mut params = screening_params();
        params.min_dividend_growth_years = Some(years);
        assert_eq!(
            validate_screening_params(&params).is_ok(),
            ok,
            "min_dividend_growth_years {years}"
        );
    }
}

/// `(除息日, 第幾個交易日填息, 觀察期滿日)`。
type Event = (&'static str, Option<i16>, Option<&'static str>);

/// 以固定查詢日 1988-06-10 執行（近 5 年是除息日晚於 1983-06-11）：
/// - DA：1985～1987 年各一次（第 10、20、30 天填息），1988-05-01 第 5 天填息但觀察期 10-01 才滿（不算）；
///   填息率 100%、平均 20 天、連續 4 年（1985～1988）。
/// - DB：1984（10 天）、1986（50 天）、1987（未填息）；填息率 66.7%、平均 30 天、1985 年沒配息所以連續 2 年。
/// - DC：只有 1980～1982 年配息（都在 5 年外）；沒有填息率、連續年數 0（最近一次早於 1987 年）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_dividend_fill_history() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_dividend_fill_history：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM dividend_fill_event WHERE stock_symbol LIKE '79979D_'",
            "DELETE FROM stocks WHERE stock_symbol IN ('79979DA', '79979DB', '79979DC')",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    let events: [(&str, &[Event]); 3] = [
        (
            "79979DA",
            &[
                ("1985-07-01", Some(10), Some("1985-12-20")),
                ("1986-07-01", Some(20), Some("1986-12-20")),
                ("1987-07-01", Some(30), Some("1987-12-20")),
                ("1988-05-01", Some(5), Some("1988-10-01")),
            ],
        ),
        (
            "79979DB",
            &[
                ("1984-07-01", Some(10), Some("1984-12-20")),
                ("1986-07-01", Some(50), Some("1986-12-20")),
                ("1987-07-01", None, Some("1987-12-20")),
            ],
        ),
        (
            "79979DC",
            &[
                ("1980-07-01", Some(10), Some("1980-12-20")),
                ("1981-07-01", Some(10), Some("1981-12-20")),
                ("1982-07-01", Some(10), Some("1982-12-20")),
            ],
        ),
    ];
    for (symbol, rows) in events {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '存股選股', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        for (ex_date, days_to_fill, mature_date) in rows {
            sqlx::query(
                "INSERT INTO dividend_fill_event (stock_symbol, ex_date, cash_dividend, pre_ex_close, days_to_fill, mature_date)
                 VALUES ($1, $2::date, 1, 20, $3, $4::date)",
            )
            .bind(symbol)
            .bind(ex_date)
            .bind(days_to_fill)
            .bind(mature_date)
            .execute(pool)
            .await
            .expect("種入填息事件");
        }
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |configure: fn(&mut StockScreeningParams)| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.limit = Some(50);
        configure(&mut params);
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
                    .is_some_and(|symbol| symbol.starts_with("79979D"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    stock["fill_rate_percent"].clone(),
                    stock["avg_fill_days"].clone(),
                    stock["dividend_years"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };

    let all = screen(|_| {}).await;
    let fill_60 = screen(|params| params.min_fill_rate_percent = Some(60.0)).await;
    let fill_80 = screen(|params| params.min_fill_rate_percent = Some(80.0)).await;
    let fast = screen(|params| params.max_avg_fill_days = Some(25.0)).await;
    let years_2 = screen(|params| params.min_dividend_years = Some(2)).await;
    let years_3 = screen(|params| params.min_dividend_years = Some(3)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            ("79979DA".to_owned(), json!(100.0), json!(20.0), json!(4)),
            ("79979DB".to_owned(), json!(66.7), json!(30.0), json!(2)),
            (
                "79979DC".to_owned(),
                serde_json::Value::Null,
                serde_json::Value::Null,
                json!(0)
            ),
        ]
    );
    let symbols = |rows: &[(
        String,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
    )]| { rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>() };
    assert_eq!(symbols(&fill_60), ["79979DA", "79979DB"]);
    assert_eq!(symbols(&fill_80), ["79979DA"]);
    assert_eq!(symbols(&fast), ["79979DA"]);
    assert_eq!(symbols(&years_2), ["79979DA", "79979DB"]);
    assert_eq!(symbols(&years_3), ["79979DA"]);
}

/// 以固定查詢日 1988-06-10 執行（只看 1987 年以前的完整年度，1988 年的除息不算）：
/// - GA：1983 1 元、1984 1 元、1985 1.5 元、1986 兩次合計 1.5 元、1987 2 元，1988-05 0.1 元不算；4 年。
/// - GB：1984 2 元、1985 1 元（減少）、1986 1.2 元、1987 1.5 元；2 年。
/// - GC：1984、1985 各 1 元、1986 沒配、1987 2 元；年度不相連，0 年。
/// - GD：1985、1986 各 1 元後停止配息（1987 年沒有）；0 年。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_computes_dividend_growth_years() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_computes_dividend_growth_years：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM dividend_fill_event WHERE stock_symbol LIKE '79979G_'",
            "DELETE FROM stocks WHERE stock_symbol IN ('79979GA', '79979GB', '79979GC', '79979GD')",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    let events: [(&str, &[(&str, &str)]); 4] = [
        (
            "79979GA",
            &[
                ("1983-07-01", "1"),
                ("1984-07-01", "1"),
                ("1985-07-01", "1.5"),
                ("1986-03-01", "0.5"),
                ("1986-09-01", "1"),
                ("1987-07-01", "2"),
                ("1988-05-01", "0.1"),
            ],
        ),
        (
            "79979GB",
            &[
                ("1984-07-01", "2"),
                ("1985-07-01", "1"),
                ("1986-07-01", "1.2"),
                ("1987-07-01", "1.5"),
            ],
        ),
        (
            "79979GC",
            &[
                ("1984-07-01", "1"),
                ("1985-07-01", "1"),
                ("1987-07-01", "2"),
            ],
        ),
        ("79979GD", &[("1985-07-01", "1"), ("1986-07-01", "1")]),
    ];
    for (symbol, rows) in events {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '股利成長', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        for (ex_date, cash) in rows {
            sqlx::query(
                "INSERT INTO dividend_fill_event (stock_symbol, ex_date, cash_dividend)
                 VALUES ($1, $2::date, $3::numeric)",
            )
            .bind(symbol)
            .bind(ex_date)
            .bind(cash)
            .execute(pool)
            .await
            .expect("種入現金除息");
        }
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |min_years: Option<u8>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.limit = Some(50);
        params.min_dividend_growth_years = min_years;
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
            .filter_map(|stock| {
                let symbol = stock["stock_symbol"].as_str()?;
                symbol
                    .starts_with("79979G")
                    .then(|| (symbol.to_owned(), stock["dividend_growth_years"].clone()))
            })
            .collect::<Vec<_>>()
    };

    let all = screen(None).await;
    let growth_2 = screen(Some(2)).await;
    let growth_3 = screen(Some(3)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            ("79979GA".to_owned(), json!(4)),
            ("79979GB".to_owned(), json!(2)),
            ("79979GC".to_owned(), json!(0)),
            ("79979GD".to_owned(), json!(0)),
        ]
    );
    assert_eq!(
        growth_2,
        [
            ("79979GA".to_owned(), json!(4)),
            ("79979GB".to_owned(), json!(2)),
        ]
    );
    assert_eq!(growth_3, [("79979GA".to_owned(), json!(4))]);
}
