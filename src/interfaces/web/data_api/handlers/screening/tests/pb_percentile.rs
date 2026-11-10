//! 條件選股：便宜價（`max_pb_percentile`、`sort_by=pb_percentile`）。

use serde_json::json;

use super::*;

/// `(代號, 分位日期, 百分位)`；查詢日 1988-06-10。VA 15、VB 40（10 天內較早的一筆）、VC 80；
/// VD 只有超過 10 天前的資料。
const PERCENTILES: [(&str, &str, i32); 4] = [
    ("79979VA", "1988-06-10", 15),
    ("79979VB", "1988-06-01", 40),
    ("79979VC", "1988-06-10", 80),
    ("79979VD", "1988-05-20", 5),
];

/// 分位上限或依分位排序單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_pb_percentile_counts_as_filter() {
    let mut params = screening_params();
    params.max_pb_percentile = Some(20.0);
    assert!(validate_screening_params(&params).is_ok());
    params.max_pb_percentile = Some(101.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("max_pb_percentile 必須介於 0 至 100")
    );
    let mut params = screening_params();
    params.sort_by = Some("pb_percentile".to_owned());
    assert!(validate_screening_params(&params).is_ok());
}

/// 每檔附查詢日前 10 天內最新一筆股價淨值比分位；依分位排序 VA、VB、VC，VD 過期排最後；
/// `max_pb_percentile=30` 只留 VA。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_and_sorts_by_pb_percentile() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_and_sorts_by_pb_percentile：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_pb_percentile WHERE stock_symbol LIKE '79979V_'",
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979V_'"#,
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979V_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, date, percentile) in PERCENTILES {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '便宜價', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ('1988-06-10', $1, 10)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入日線");
        sqlx::query(
            "INSERT INTO daily_pb_percentile (date, stock_symbol, pb, pb_percentile, samples)
             VALUES ($1::date, $2, 1, $3, 1000)",
        )
        .bind(date)
        .bind(symbol)
        .bind(Decimal::from(percentile))
        .execute(pool)
        .await
        .expect("種入淨值比分位");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |max_pb_percentile: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.max_pb_percentile = max_pb_percentile;
        params.limit = Some(50);
        params.sort_by = Some("pb_percentile".to_owned());
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
                    .is_some_and(|symbol| symbol.starts_with("79979V"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["pb_percentile"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let all = screen(None).await;
    let cheap = screen(Some(30.0)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            (json!("79979VA"), json!(15.0)),
            (json!("79979VB"), json!(40.0)),
            (json!("79979VC"), json!(80.0)),
            (json!("79979VD"), json!(null)),
        ]
    );
    assert_eq!(cheap, [(json!("79979VA"), json!(15.0))]);
}
