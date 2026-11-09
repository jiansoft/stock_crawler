//! 條件選股：均值回歸（`max_bias240`、`sort_by=bias240`）。

use serde_json::json;

use super::*;

/// `(代號, 日線日期, 收盤, 年線)`；查詢日 1988-06-10。MA 乖離 -20%、MB -5%（前一天的日線）、MC +10%；MD 年線為 0。
const QUOTES: [(&str, &str, i32, i32); 4] = [
    ("79979MA", "1988-06-10", 80, 100),
    ("79979MB", "1988-06-09", 95, 100),
    ("79979MC", "1988-06-10", 110, 100),
    ("79979MD", "1988-06-10", 50, 0),
];

/// 乖離上限或依乖離排序單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_bias240_counts_as_filter() {
    let mut params = screening_params();
    params.max_bias240 = Some(-15.0);
    assert!(validate_screening_params(&params).is_ok());
    params.max_bias240 = Some(-91.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("max_bias240 必須介於 -90 至 100")
    );
    let mut params = screening_params();
    params.sort_by = Some("bias240".to_owned());
    assert!(validate_screening_params(&params).is_ok());
}

/// 每檔附查詢日前 10 天內最新一筆日線的年線乖離率；依乖離排序 MA、MB、MC，MD 沒有年線排最後；
/// `max_bias240=-10` 只留 MA。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_and_sorts_by_bias240() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_and_sorts_by_bias240：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979M_'"#,
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979M_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, date, close, ma240) in QUOTES {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '均值回歸', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "MovingAverage240")
               VALUES ($1::date, $2, $3, $4)"#,
        )
        .bind(date)
        .bind(symbol)
        .bind(Decimal::from(close))
        .bind(Decimal::from(ma240))
        .execute(pool)
        .await
        .expect("種入日線");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |max_bias240: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.max_bias240 = max_bias240;
        params.limit = Some(50);
        params.sort_by = Some("bias240".to_owned());
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
                    .is_some_and(|symbol| symbol.starts_with("79979M"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["bias240_percent"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let all = screen(None).await;
    let deep = screen(Some(-10.0)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            (json!("79979MA"), json!(-20.0)),
            (json!("79979MB"), json!(-5.0)),
            (json!("79979MC"), json!(10.0)),
            (json!("79979MD"), json!(null)),
        ]
    );
    assert_eq!(deep, [(json!("79979MA"), json!(-20.0))]);
}
