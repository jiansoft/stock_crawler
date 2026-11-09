//! 條件選股：籌碼集中（400 張以上大戶持股比例變化、融資餘額變化）。

use super::*;

/// 兩個條件的範圍邊界，且單獨即可成立篩選。
#[test]
fn screening_chip_concentration_boundaries() {
    for (value, ok) in [
        (-100.0, true),
        (100.0, true),
        (-100.5, false),
        (100.5, false),
    ] {
        let mut params = screening_params();
        params.min_holder_400_change = Some(value);
        assert_eq!(
            validate_screening_params(&params).is_ok(),
            ok,
            "min_holder_400_change {value}"
        );
    }
    for (value, ok) in [
        (-100.0, true),
        (1000.0, true),
        (-101.0, false),
        (1000.5, false),
    ] {
        let mut params = screening_params();
        params.max_margin_change_percent = Some(value);
        assert_eq!(
            validate_screening_params(&params).is_ok(),
            ok,
            "max_margin_change_percent {value}"
        );
    }
}

/// 以固定歷史查詢日 1988-03-05（週六）執行：
/// - 400 張大戶：CA 03-04 對 02-26 為 +2（03-05 當天的週不會提前用到）、CB 為 -1。
/// - 融資：CA 02-04～02-24 共 21 筆，最新 02-24 為 800、往前第 20 筆 02-04 為 1000（-20%）；CB 全部 1000（0%）；
///   CC 融資為 0（沒有資料）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_chip_concentration() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_chip_concentration：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM holder_distribution WHERE stock_symbol LIKE '79979C_'",
            "DELETE FROM chip_daily WHERE stock_symbol LIKE '79979C_'",
            "DELETE FROM stocks WHERE stock_symbol IN ('79979CA', '79979CB', '79979CC')",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, percents, margins) in [
        ("79979CA", [60, 61, 63, 40], (1000, 800)),
        ("79979CB", [60, 61, 60, 90], (1000, 1000)),
        ("79979CC", [60, 60, 60, 60], (0, 0)),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '籌碼集中', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        for (date, percent) in ["1988-02-19", "1988-02-26", "1988-03-04", "1988-03-05"]
            .into_iter()
            .zip(percents)
        {
            sqlx::query(
                r#"INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders, percent_400)
                   VALUES ($1::date, $2, 10, 50, 100, $3)"#,
            )
            .bind(date)
            .bind(symbol)
            .bind(Decimal::from(percent))
            .execute(pool)
            .await
            .expect("種入集保週資料");
        }
        // 02-04 為基期，02-05～02-24 為之後的 20 筆。
        for offset in 0..21 {
            let date = NaiveDate::from_ymd_opt(1988, 2, 4).expect("固定日期合法")
                + chrono::Duration::days(offset);
            let balance = if offset == 0 { margins.0 } else { margins.1 };
            sqlx::query(
                r#"INSERT INTO chip_daily ("date", stock_symbol, margin_balance) VALUES ($1, $2, $3)"#,
            )
            .bind(date)
            .bind(symbol)
            .bind(i64::from(balance))
            .execute(pool)
            .await
            .expect("種入融資餘額");
        }
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
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
            .filter_map(|stock| {
                let symbol = stock["stock_symbol"].as_str()?;
                symbol
                    .starts_with("79979C")
                    .then(|| (symbol.to_owned(), stock["margin_change_percent"].clone()))
            })
            .collect::<Vec<_>>()
    };

    let holder_up = screen(|params| {
        params.min_holder_400_change = Some(1.5);
        params.major_change_weeks = Some(1);
    })
    .await;
    let holder_any = screen(|params| {
        params.min_holder_400_change = Some(-1.0);
        params.major_change_weeks = Some(1);
    })
    .await;
    let margin_down = screen(|params| params.max_margin_change_percent = Some(-10.0)).await;
    let margin_flat = screen(|params| params.max_margin_change_percent = Some(0.0)).await;
    let plain = screen(|_| {}).await;
    cleanup().await;

    let symbols = |rows: &[(String, serde_json::Value)]| {
        rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>()
    };
    assert_eq!(symbols(&holder_up), ["79979CA"]);
    assert_eq!(symbols(&holder_any), ["79979CA", "79979CB", "79979CC"]);
    assert_eq!(
        margin_down,
        [("79979CA".to_owned(), serde_json::json!(-20.0))]
    );
    assert_eq!(
        margin_flat,
        [
            ("79979CA".to_owned(), serde_json::json!(-20.0)),
            ("79979CB".to_owned(), serde_json::json!(0.0)),
        ]
    );
    assert!(
        plain.iter().all(|row| row.1.is_null()),
        "沒設條件時不算融資變化：{plain:?}"
    );
}
