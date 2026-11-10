//! 條件選股：安心存股（`min_market_cap`、`min_profit_years`）。

use serde_json::json;

use super::*;

/// 測試股票代號（查詢日 1988-06-10，收盤皆 50 元）。
const SYMBOLS: [&str; 3] = ["79979WA", "79979WB", "79979WC"];

/// 市值或連續獲利年數下限單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_steady_filters_count_as_filter() {
    let mut params = screening_params();
    params.min_market_cap = Some(100.0);
    assert!(validate_screening_params(&params).is_ok());
    params.min_market_cap = Some(0.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_market_cap 必須介於 1 至 1000000")
    );
    let mut params = screening_params();
    params.min_profit_years = Some(5);
    assert!(validate_screening_params(&params).is_ok());
    params.min_profit_years = Some(31);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_profit_years 必須介於 1 至 30")
    );
}

/// 每檔附市值（億元）與連續獲利年數：WA 10 億股 → 500 億、1985～1987 皆賺 → 3 年；WB 1 億股 → 50 億、1986 虧
/// 1987 賺 → 1 年（1988 年度尚未公布）；WC 沒有發行股數、最新年度 1985 落後太久 → 市值 null、0 年。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_by_market_cap_and_profit_years() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_by_market_cap_and_profit_years：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM financial_statement WHERE security_code = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOLS).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, issued) in SYMBOLS.into_iter().zip([1_000_000_000_i64, 100_000_000, 0]) {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, issued_share)
               VALUES ($1, '安心存股', $1, 2, 1, $2)"#,
        )
        .bind(symbol)
        .bind(issued)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
               VALUES ('1988-06-10', $1, 50)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入日線");
    }
    sqlx::query(
        "INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share)
         VALUES ('79979WA', 1985, '', 2), ('79979WA', 1986, '', 2), ('79979WA', 1987, '', 2),
                ('79979WB', 1986, '', -1), ('79979WB', 1987, '', 1), ('79979WB', 1988, '', 5),
                ('79979WC', 1984, '', 1), ('79979WC', 1985, '', 1)",
    )
    .execute(pool)
    .await
    .expect("種入年度 EPS");

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |min_market_cap: Option<f64>, min_profit_years: Option<u8>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_market_cap = min_market_cap;
        params.min_profit_years = min_profit_years;
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
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["market_cap"].clone(),
                    stock["profit_years"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let profitable = screen(None, Some(1)).await;
    let large = screen(Some(100.0), None).await;
    let steady = screen(None, Some(3)).await;
    cleanup().await;

    assert_eq!(
        profitable,
        [
            (json!("79979WA"), json!(500.0), json!(3)),
            (json!("79979WB"), json!(50.0), json!(1)),
        ]
    );
    assert_eq!(large, [(json!("79979WA"), json!(500.0), json!(3))]);
    assert_eq!(steady, large);
}
