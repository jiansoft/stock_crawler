//! 條件選股：長期高 ROE（`min_roe_years`、`min_annual_roe_percent`）。

use serde_json::json;

use super::*;

/// 測試股票代號（查詢日 1988-06-10，已公布的最新年度是 1987）。
const SYMBOLS: [&str; 4] = ["79979HRA", "79979HRB", "79979HRC", "79979HRD"];

/// 只設門檻即成立篩選；年數須搭配門檻，兩者超出範圍回錯誤。
#[test]
fn screening_roe_years_validation() {
    let mut params = screening_params();
    params.min_annual_roe_percent = Some(15.0);
    let validated = validate_screening_params(&params).expect("只設門檻合法");
    assert_eq!(validated.min_roe_years, Some(1));
    params.min_roe_years = Some(5);
    let validated = validate_screening_params(&params).expect("年數加門檻合法");
    assert_eq!(validated.min_roe_years, Some(5));
    params.min_roe_years = Some(11);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_roe_years 必須介於 1 至 10")
    );
    params.min_roe_years = Some(5);
    params.min_annual_roe_percent = Some(101.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_annual_roe_percent 必須介於 0 至 100")
    );

    let mut params = screening_params();
    params.min_roe_years = Some(5);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_roe_years 需搭配 min_annual_roe_percent")
    );
}

/// 年度 ROE（%）：HRA 1985～1987 為 20、18、16 → ≥ 15 連續 3 年；HRB 20、10、25 → 1 年；HRC 最新年度 1984
/// 落後太久 → 0 年；HRD 1986～1987 皆 30 → 2 年（1988 年度尚未公布，不算）。沒設門檻時 `roe_years` 為 `null`。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_by_roe_years() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_by_roe_years：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM financial_statement WHERE security_code = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOLS).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '長期高ROE', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share, return_on_equity)
         VALUES ('79979HRA', 1985, '', 1, 20), ('79979HRA', 1986, '', 1, 18), ('79979HRA', 1987, '', 1, 16),
                ('79979HRB', 1985, '', 1, 20), ('79979HRB', 1986, '', 1, 10), ('79979HRB', 1987, '', 1, 25),
                ('79979HRC', 1983, '', 1, 30), ('79979HRC', 1984, '', 1, 30),
                ('79979HRD', 1986, '', 1, 30), ('79979HRD', 1987, '', 1, 30), ('79979HRD', 1988, '', 1, 5)",
    )
    .execute(pool)
    .await
    .expect("種入年度 ROE");

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |threshold: Option<f64>, years: Option<u8>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_annual_roe_percent = threshold;
        params.min_roe_years = years;
        params.limit = Some(50);
        params.sort_by = Some("stock_symbol".to_owned());
        params.sort_order = Some("asc".to_owned());
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
            .map(|stock| (stock["stock_symbol"].clone(), stock["roe_years"].clone()))
            .collect::<Vec<_>>()
    };
    let unfiltered = screen(None, None).await;
    let latest = screen(Some(15.0), None).await;
    let steady = screen(Some(15.0), Some(2)).await;
    cleanup().await;

    assert_eq!(
        unfiltered,
        SYMBOLS
            .iter()
            .map(|symbol| (json!(symbol), json!(null)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        latest,
        [
            (json!("79979HRA"), json!(3)),
            (json!("79979HRB"), json!(1)),
            (json!("79979HRD"), json!(2)),
        ]
    );
    assert_eq!(
        steady,
        [(json!("79979HRA"), json!(3)), (json!("79979HRD"), json!(2))]
    );
}
