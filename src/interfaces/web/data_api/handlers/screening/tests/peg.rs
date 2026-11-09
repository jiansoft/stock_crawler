//! 條件選股：PEG（`max_peg`、`sort_by=peg`）與近四季 EPS 成長率。

use serde_json::json;

use super::*;

/// 1986 Q3～1988 Q2（舊到新）；查詢日 1988-08-10 時最新一季 1988 Q2 在兩季內。
const QUARTERS: [(i32, &str); 8] = [
    (1986, "Q3"),
    (1986, "Q4"),
    (1987, "Q1"),
    (1987, "Q2"),
    (1987, "Q3"),
    (1987, "Q4"),
    (1988, "Q1"),
    (1988, "Q2"),
];

/// `(代號, 前四季每季 EPS, 近四季每季 EPS, 季數, 本益比)`：
/// - PA：1 → 2，成長 100%、本益比 20 → PEG 0.2。
/// - PB：2 → 3，成長 50%、本益比 30 → PEG 0.6。
/// - PC：2 → 1，成長 −50%，沒有 PEG。
/// - PD：只有 7 季，成長率與 PEG 皆為 `null`。
const STOCKS: [(&str, i32, i32, usize, i32); 4] = [
    ("79979PA", 1, 2, 8, 20),
    ("79979PB", 2, 3, 8, 30),
    ("79979PC", 2, 1, 8, 10),
    ("79979PD", 1, 2, 7, 20),
];

/// `max_peg` 單獨即可成立篩選，範圍 0.01–10。
#[test]
fn screening_max_peg_counts_as_filter() {
    let mut params = screening_params();
    params.max_peg = Some(1.0);
    assert!(validate_screening_params(&params).is_ok());
    for value in [0.0, 10.5] {
        params.max_peg = Some(value);
        assert_eq!(
            validate_screening_params(&params).err(),
            Some("max_peg 必須介於 0.01 至 10"),
            "{value}"
        );
    }
}

/// 每檔回傳近四季 EPS 成長率與 PEG；`max_peg=0.5` 只留 PA，依 PEG 排序為 PA、PB，沒有 PEG 的排最後。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_and_sorts_by_peg() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_and_sorts_by_peg：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979P_'"#,
            "DELETE FROM income_statement WHERE stock_symbol LIKE '79979P_'",
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979P_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, previous, recent, quarters, pe) in STOCKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, 'PEG', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        for (index, (year, quarter)) in QUARTERS.into_iter().enumerate().skip(8 - quarters) {
            let eps = if index >= 4 { recent } else { previous };
            sqlx::query(
                "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, eps)
                 VALUES ($1, $2, 'single', $3, 'yahoo', $4)",
            )
            .bind(symbol)
            .bind(year)
            .bind(quarter)
            .bind(Decimal::from(eps))
            .execute(pool)
            .await
            .expect("種入損益表");
        }
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "PriceEarningRatio", "TradeValue")
               VALUES ('1988-08-10', $1, $2, 2000000)"#,
        )
        .bind(symbol)
        .bind(Decimal::from(pe))
        .execute(pool)
        .await
        .expect("種入日線");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 8, 10).expect("固定日期合法");
    let screen = |max_peg: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_avg_trade_value = Some(1);
        params.max_peg = max_peg;
        params.limit = Some(50);
        params.sort_by = Some("peg".to_owned());
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
                    .is_some_and(|symbol| symbol.starts_with("79979P"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["eps_ttm_growth_percent"].clone(),
                    stock["peg_ratio"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let all = screen(None).await;
    let filtered = screen(Some(0.5)).await;
    cleanup().await;

    let null = serde_json::Value::Null;
    assert_eq!(
        all,
        [
            (json!("79979PA"), json!(100.0), json!(0.2)),
            (json!("79979PB"), json!(50.0), json!(0.6)),
            (json!("79979PC"), json!(-50.0), null.clone()),
            (json!("79979PD"), null.clone(), null),
        ]
    );
    assert_eq!(filtered, [(json!("79979PA"), json!(100.0), json!(0.2))]);
}
