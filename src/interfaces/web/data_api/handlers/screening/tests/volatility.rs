//! 條件選股：低波動（`max_volatility_percent`、`max_beta`、`sort_by=volatility`／`beta`）。

use serde_json::json;

use super::*;

/// `(代號, 指標日期, 年化波動度, beta)`；查詢日 1988-06-10。VD 的指標是 05-20（超過 10 天，視為沒有資料）。
const RISKS: [(&str, &str, i32, i32); 4] = [
    ("79979VA", "1988-06-10", 15, 1),
    ("79979VB", "1988-06-09", 45, 2),
    ("79979VC", "1988-06-10", 25, -1),
    ("79979VD", "1988-05-20", 10, 0),
];

/// 兩個上限單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_volatility_caps_count_as_filters() {
    let mut params = screening_params();
    params.max_volatility_percent = Some(20.0);
    assert!(validate_screening_params(&params).is_ok());
    params.max_volatility_percent = Some(0.5);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("max_volatility_percent 必須介於 1 至 200")
    );
    let mut params = screening_params();
    params.max_beta = Some(0.8);
    assert!(validate_screening_params(&params).is_ok());
    params.max_beta = Some(5.5);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("max_beta 必須介於 -5 至 5")
    );
}

/// 每檔附查詢日前 10 天內最新的年化波動度與 beta；依波動度排序 VA、VC、VB，VD 過期沒有資料排最後；
/// `max_volatility_percent=30` 留 VA、VC，`max_beta=0` 只留 VC。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_and_sorts_by_volatility() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_and_sorts_by_volatility：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_risk_metric WHERE stock_symbol LIKE '79979V_'",
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979V_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, date, volatility, beta) in RISKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '低波動', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            "INSERT INTO daily_risk_metric (date, stock_symbol, volatility_percent, beta, samples)
             VALUES ($1::date, $2, $3, $4, 240)",
        )
        .bind(date)
        .bind(symbol)
        .bind(Decimal::from(volatility))
        .bind(Decimal::from(beta))
        .execute(pool)
        .await
        .expect("種入風險指標");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |max_volatility: Option<f64>, max_beta: Option<f64>, sort_by: &'static str| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.max_volatility_percent = max_volatility.or(Some(200.0));
        params.max_beta = max_beta;
        params.limit = Some(50);
        params.sort_by = Some(sort_by.to_owned());
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
                    stock["volatility_percent"].clone(),
                    stock["beta"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    // 上限 200 時資料齊全的三檔都通過，VD 過期沒有資料、不通過。
    let all = screen(None, None, "volatility").await;
    let by_beta = screen(None, None, "beta").await;
    let calm = screen(Some(30.0), None, "volatility").await;
    let defensive = screen(None, Some(0.0), "volatility").await;
    cleanup().await;

    assert_eq!(
        all,
        [
            (json!("79979VA"), json!(15.0), json!(1.0)),
            (json!("79979VC"), json!(25.0), json!(-1.0)),
            (json!("79979VB"), json!(45.0), json!(2.0)),
        ]
    );
    let order: Vec<_> = by_beta.iter().map(|row| row.0.clone()).collect();
    assert_eq!(
        order,
        [json!("79979VC"), json!("79979VA"), json!("79979VB")]
    );
    let calm: Vec<_> = calm.iter().map(|row| row.0.clone()).collect();
    assert_eq!(calm, [json!("79979VA"), json!("79979VC")]);
    let defensive: Vec<_> = defensive.iter().map(|row| row.0.clone()).collect();
    assert_eq!(defensive, [json!("79979VC")]);
}
