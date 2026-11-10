//! 條件選股：研發成長股（`min_rd_ratio_percent`、`sort_by=rd_ratio`）。

use serde_json::json;

use super::*;

/// 查詢日 1988-06-10（1988 Q2）：DA、DB 最近四季為 1987 Q2～1988 Q1（研發 8%、3%）；DC 最新一季是 1987 Q3
/// （超過兩季、過期）；DD 缺 1987 Q4（季別不相連）。
const ROWS: [(&str, i32, &str, i32); 15] = [
    ("79979DA", 1987, "Q2", 8),
    ("79979DA", 1987, "Q3", 8),
    ("79979DA", 1987, "Q4", 8),
    ("79979DA", 1988, "Q1", 8),
    ("79979DB", 1987, "Q2", 3),
    ("79979DB", 1987, "Q3", 3),
    ("79979DB", 1987, "Q4", 3),
    ("79979DB", 1988, "Q1", 3),
    ("79979DC", 1986, "Q4", 9),
    ("79979DC", 1987, "Q1", 9),
    ("79979DC", 1987, "Q2", 9),
    ("79979DC", 1987, "Q3", 9),
    ("79979DD", 1987, "Q2", 9),
    ("79979DD", 1987, "Q3", 9),
    ("79979DD", 1988, "Q1", 9),
];

/// 研發費用率下限或依研發費用率排序單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_rd_ratio_counts_as_filter() {
    let mut params = screening_params();
    params.min_rd_ratio_percent = Some(5.0);
    assert!(validate_screening_params(&params).is_ok());
    params.min_rd_ratio_percent = Some(101.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_rd_ratio_percent 必須介於 0 至 100")
    );
}

/// 每檔附近四季研發費用率；依研發費用率降冪為 DA、DB，DC 過期、DD 不相連為 `null` 排最後；
/// `min_rd_ratio_percent=5` 只留 DA。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_and_sorts_by_rd_ratio() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_and_sorts_by_rd_ratio：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM income_statement WHERE stock_symbol LIKE '79979D_'",
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979D_'"#,
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979D_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in ["79979DA", "79979DB", "79979DC", "79979DD"] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '研發成長', $1, 2, 1)"#,
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
    }
    for (symbol, year, quarter, rd) in ROWS {
        sqlx::query(
            "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, rd_expenses)
             VALUES ($1, $2, 'single', $3, 'yahoo', 100, $4)",
        )
        .bind(symbol)
        .bind(year)
        .bind(quarter)
        .bind(i64::from(rd))
        .execute(pool)
        .await
        .expect("種入損益表");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |min_rd_ratio_percent: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_rd_ratio_percent = min_rd_ratio_percent;
        params.limit = Some(50);
        params.sort_by = Some("rd_ratio".to_owned());
        params.sort_order = Some("desc".to_owned());
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
                    .is_some_and(|symbol| symbol.starts_with("79979D"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["rd_ratio_percent"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let all = screen(None).await;
    let heavy = screen(Some(5.0)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            (json!("79979DA"), json!(8.0)),
            (json!("79979DB"), json!(3.0)),
            (json!("79979DC"), json!(null)),
            (json!("79979DD"), json!(null)),
        ]
    );
    assert_eq!(heavy, [(json!("79979DA"), json!(8.0))]);
}
