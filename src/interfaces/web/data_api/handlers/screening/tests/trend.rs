//! 條件選股：Minervini 趨勢模板（`trend_template`）。

use serde_json::json;

use super::*;

/// 種入 1988-06-10（查詢日）的日線；TE 只有 05-20 的日線（超過 10 天，視為沒有資料）。
/// `(代號, 日期, 收盤, MA20, MA60, MA120, MA240, 52 週最高, 52 週最低)`：
/// - TA：全部條件成立。
/// - TB：MA20 低於 MA60（均線沒有多頭排列）。
/// - TC：收盤 100 低於 52 週最高 140 的 75%（105）。
/// - TD：52 週最低價為 0（資料不足）。
const QUOTES: [(&str, &str, [i32; 7]); 5] = [
    ("79979TA", "1988-06-10", [100, 95, 90, 85, 80, 110, 60]),
    ("79979TB", "1988-06-10", [100, 85, 90, 85, 80, 110, 60]),
    ("79979TC", "1988-06-10", [100, 95, 90, 85, 80, 140, 60]),
    ("79979TD", "1988-06-10", [100, 95, 90, 85, 80, 110, 0]),
    ("79979TE", "1988-05-20", [100, 95, 90, 85, 80, 110, 60]),
];

/// `trend_template` 單獨即可成立篩選；`false` 不算篩選條件。
#[test]
fn screening_trend_template_counts_as_filter() {
    let mut params = screening_params();
    params.trend_template = Some(true);
    assert!(validate_screening_params(&params).is_ok());
    params.trend_template = Some(false);
    assert!(validate_screening_params(&params).is_err());
}

/// 每檔回傳 `trend_template`（資料不足或日線過期為 `null`），設 `trend_template=true` 時只留符合的股票。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_applies_trend_template() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_applies_trend_template：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979T_'"#,
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979T_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, date, [close, ma20, ma60, ma120, ma240, high, low]) in QUOTES {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '趨勢模板', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "MovingAverage20", "MovingAverage60",
                   "MovingAverage120", "MovingAverage240", maximum_price_in_year, minimum_price_in_year)
               VALUES ($1::date, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(date)
        .bind(symbol)
        .bind(Decimal::from(close))
        .bind(Decimal::from(ma20))
        .bind(Decimal::from(ma60))
        .bind(Decimal::from(ma120))
        .bind(Decimal::from(ma240))
        .bind(Decimal::from(high))
        .bind(Decimal::from(low))
        .execute(pool)
        .await
        .expect("種入日線");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |trend_template: Option<bool>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.limit = Some(50);
        params.trend_template = trend_template;
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
                    .starts_with("79979T")
                    .then(|| (symbol.to_owned(), stock["trend_template"].clone()))
            })
            .collect::<Vec<_>>()
    };

    let all = screen(None).await;
    let passed = screen(Some(true)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            ("79979TA".to_owned(), json!(true)),
            ("79979TB".to_owned(), json!(false)),
            ("79979TC".to_owned(), json!(false)),
            ("79979TD".to_owned(), serde_json::Value::Null),
            ("79979TE".to_owned(), serde_json::Value::Null),
        ]
    );
    assert_eq!(passed, [("79979TA".to_owned(), json!(true))]);
}
