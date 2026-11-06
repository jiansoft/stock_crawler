//! 條件選股：CANSLIM 的 C（單季 EPS 年增）、A（年度 EPS 成長）、N（52 週新高）條件。

use super::*;

/// 三個條件各自的範圍邊界，且單獨即可成立篩選。
#[test]
fn screening_canslim_boundaries() {
    for (value, ok) in [
        (-100.0, true),
        (10_000.0, true),
        (-100.5, false),
        (10_001.0, false),
    ] {
        let mut params = screening_params();
        params.min_eps_yoy_percent = Some(value);
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "min_eps_yoy_percent {value}");

        let mut params = screening_params();
        params.min_annual_eps_growth_percent = Some(value);
        let validated = validate_screening_params(&params);
        assert_eq!(
            validated.is_ok(),
            ok,
            "min_annual_eps_growth_percent {value}"
        );
    }
    let mut params = screening_params();
    params.min_eps_yoy_percent = Some(f64::NAN);
    assert!(validate_screening_params(&params).is_err(), "NaN 必須拒絕");

    for (days, ok) in [(1, true), (365, true), (0, false), (366, false)] {
        let mut params = screening_params();
        params.new_high_days = Some(days);
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "new_high_days {days}");
        if let Ok(validated) = validated {
            assert_eq!(validated.new_high_days, Some(i32::from(days)));
        }
    }
}

/// 以固定歷史查詢日 1988-06-10（最新季報 1988-Q1、最新年度 1987）執行：
/// - CA：單季 EPS 1.0 → 1.5（+50%）；年度 1.0、1.3、1.7、2.2（三年最低 +29.41%）；06-01 創 52 週新高。
/// - CB：去年同季虧損（無法算年增）；年度三年最低 +7.69%；03-01 創新高（30 天外、120 天內）。
/// - CC：單季 +20%；年度不連續（缺 1984）不算成長率；沒有新高。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_canslim_growth_and_new_highs() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_canslim_growth_and_new_highs：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979C_'",
            "DELETE FROM financial_statement WHERE security_code LIKE '79979C_'",
            "DELETE FROM stocks WHERE stock_symbol IN ('79979CA', '79979CB', '79979CC')",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in ["79979CA", "79979CB", "79979CC"] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, 'CANSLIM 選股', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    let statements = [
        ("79979CA", 1987, "Q1", "1.0"),
        ("79979CA", 1988, "Q1", "1.5"),
        ("79979CA", 1984, "", "1.0"),
        ("79979CA", 1985, "", "1.3"),
        ("79979CA", 1986, "", "1.7"),
        ("79979CA", 1987, "", "2.2"),
        ("79979CB", 1987, "Q1", "-0.5"),
        ("79979CB", 1988, "Q1", "1.0"),
        ("79979CB", 1984, "", "1.0"),
        ("79979CB", 1985, "", "1.3"),
        ("79979CB", 1986, "", "1.4"),
        ("79979CB", 1987, "", "2.0"),
        ("79979CC", 1987, "Q1", "1.0"),
        ("79979CC", 1988, "Q1", "1.2"),
        ("79979CC", 1983, "", "1.0"),
        ("79979CC", 1985, "", "1.5"),
        ("79979CC", 1986, "", "2.0"),
        ("79979CC", 1987, "", "3.0"),
    ];
    for (symbol, year, quarter, eps) in statements {
        sqlx::query(
            "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share)
             VALUES ($1, $2, $3, $4::numeric)",
        )
        .bind(symbol)
        .bind(year)
        .bind(quarter)
        .bind(eps)
        .execute(pool)
        .await
        .expect("種入財報");
    }
    for (symbol, date) in [("79979CA", "1988-06-01"), ("79979CB", "1988-03-01")] {
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1::date, $2, '52_week', 'new_high', $2 || ' 52 週新高')",
        )
        .bind(date)
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入訊號");
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
                    .is_some_and(|symbol| symbol.starts_with("79979C"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    stock["eps_yoy_percent"].clone(),
                    stock["annual_eps_growth_percent"].clone(),
                    stock["annual_eps_year"].clone(),
                    stock["new_high_date"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };

    let any_eps_yoy = screen(|params| params.min_eps_yoy_percent = Some(-100.0)).await;
    let eps_yoy = screen(|params| params.min_eps_yoy_percent = Some(25.0)).await;
    let annual = screen(|params| params.min_annual_eps_growth_percent = Some(25.0)).await;
    let new_high_30 = screen(|params| params.new_high_days = Some(30)).await;
    let new_high_120 = screen(|params| params.new_high_days = Some(120)).await;
    cleanup().await;

    let symbols = |rows: &[(
        String,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
    )]| { rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>() };
    // 去年同季虧損的 CB 沒有年增率，連 -100% 的下限都不會通過；沒設 new_high_days 時不查新高。
    assert_eq!(symbols(&any_eps_yoy), ["79979CA", "79979CC"]);
    assert_eq!(any_eps_yoy[0].1, serde_json::json!(50.0));
    assert_eq!(any_eps_yoy[1].1, serde_json::json!(20.0));
    assert_eq!(
        any_eps_yoy[1].2,
        serde_json::Value::Null,
        "年度不連續不算成長率"
    );
    assert!(
        any_eps_yoy.iter().all(|row| row.4.is_null()),
        "{any_eps_yoy:?}"
    );

    assert_eq!(symbols(&eps_yoy), ["79979CA"]);
    assert_eq!(symbols(&annual), ["79979CA"]);
    assert_eq!(annual[0].2, serde_json::json!(29.41));
    assert_eq!(annual[0].3, serde_json::json!(1987));

    assert_eq!(symbols(&new_high_30), ["79979CA"]);
    assert_eq!(new_high_30[0].4, serde_json::json!("1988-06-01"));
    assert_eq!(symbols(&new_high_120), ["79979CA", "79979CB"]);
    assert_eq!(new_high_120[1].4, serde_json::json!("1988-03-01"));
}
