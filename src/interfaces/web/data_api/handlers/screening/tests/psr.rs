//! 條件選股：肯尼斯費雪（`max_psr`、`min_avg_net_margin_3y`）。

use serde_json::json;

use super::*;

/// 測試股票代號（查詢日 1988-06-10，收盤皆 50 元、發行股數皆 1 億股）。
const SYMBOLS: [&str; 3] = ["79979PA", "79979PB", "79979PC"];

/// 兩個條件單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_psr_filters_count_as_filter() {
    let mut params = screening_params();
    params.max_psr = Some(1.5);
    assert!(validate_screening_params(&params).is_ok());
    params.max_psr = Some(101.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("max_psr 必須介於 0 至 100")
    );
    let mut params = screening_params();
    params.min_avg_net_margin_3y = Some(-5.0);
    assert!(validate_screening_params(&params).is_ok());
    params.min_avg_net_margin_3y = Some(-101.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_avg_net_margin_3y 必須介於 -100 至 100")
    );
}

/// 每檔附 PSR 與近 3 年平均稅後淨利率：PA 每月營收 25 萬千元 → 近 12 個月 300 萬、市值 500 萬千元 → PSR 1.67，
/// 1985～1987 淨利率 10%、12%、14% → 12%（1988 年度尚未公布，不算）；PB 每月 5 萬 → PSR 8.33，5%、5%、20% → 10%；
/// PC 缺 1987-10 月營收、缺 1986 年度且 1987 營收為 0 → 兩者皆 null（不會除以零出錯）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_by_psr_and_net_margin() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_by_psr_and_net_margin：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
            r#"DELETE FROM "Revenue" WHERE "SecurityCode" = ANY($1)"#,
            "DELETE FROM income_statement WHERE stock_symbol = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOLS).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, issued_share)
               VALUES ($1, '費雪', $1, 2, 1, 100000000)"#,
        )
        .bind(symbol)
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
    for sql in [
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
           SELECT s, s, to_char(month, 'YYYYMM')::bigint, monthly
           FROM (VALUES ('79979PA', 250000), ('79979PB', 50000), ('79979PC', 50000)) AS v(s, monthly)
           CROSS JOIN generate_series('1987-05-01'::date, '1988-04-01'::date, INTERVAL '1 month') AS month
           WHERE NOT (s = '79979PC' AND month = '1987-10-01')"#,
        "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, net_income)
         VALUES ('79979PA', 1985, 'annual', 'A', 'yahoo', 1000, 100),
                ('79979PA', 1986, 'annual', 'A', 'yahoo', 1000, 120),
                ('79979PA', 1987, 'annual', 'A', 'yahoo', 1000, 140),
                ('79979PA', 1988, 'annual', 'A', 'yahoo', 1000, -500),
                ('79979PB', 1985, 'annual', 'A', 'yahoo', 1000, 50),
                ('79979PB', 1986, 'annual', 'A', 'yahoo', 1000, 50),
                ('79979PB', 1987, 'annual', 'A', 'yahoo', 1000, 200),
                ('79979PC', 1985, 'annual', 'A', 'yahoo', 1000, 300),
                ('79979PC', 1987, 'annual', 'A', 'yahoo', 0, 300)",
    ] {
        sqlx::query(sql).execute(pool).await.expect("種入費雪資料");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |max_psr: Option<f64>, min_margin: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.max_psr = max_psr;
        params.min_avg_net_margin_3y = min_margin;
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
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["psr"].clone(),
                    stock["avg_net_margin_3y_percent"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let unfiltered = screen(None, None).await;
    let cheap = screen(Some(2.0), None).await;
    let thick = screen(None, Some(11.0)).await;
    let profitable = screen(None, Some(8.0)).await;
    cleanup().await;

    let a = (json!("79979PA"), json!(1.67), json!(12.0));
    let b = (json!("79979PB"), json!(8.33), json!(10.0));
    assert_eq!(
        unfiltered,
        [
            a.clone(),
            b.clone(),
            (json!("79979PC"), json!(null), json!(null))
        ]
    );
    assert_eq!(cheap, std::slice::from_ref(&a));
    assert_eq!(thick, std::slice::from_ref(&a));
    assert_eq!(profitable, [a, b]);
}
