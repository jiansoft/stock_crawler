//! 條件選股：葛拉漢（`max_pe_pb_product`、`min_current_ratio`、`sort_by=pe_pb_product`）。

use serde_json::json;

use super::*;

/// 查詢日 1988-06-10（1988 Q2）的日線本益比／股價淨值比與最新一季資產負債表（流動資產，流動負債皆為 100）：
/// GRA 10 × 1.5、1988 Q1 流動資產 300；GRB 20 × 2、1988 Q1 150；GRC 虧損、最新一季是 1987 Q3（超過兩季、過期）；
/// GRD 8 × 1、1988 Q1 流動負債為 0。
const ROWS: [(&str, &str, &str, i32, &str, i64, i64); 4] = [
    ("79979GRA", "10", "1.5", 1988, "Q1", 300, 100),
    ("79979GRB", "20", "2", 1988, "Q1", 150, 100),
    ("79979GRC", "0", "1", 1987, "Q3", 500, 100),
    ("79979GRD", "8", "1", 1988, "Q1", 300, 0),
];

/// 乘積上限、流動比率下限或依乘積排序單獨即可成立篩選；超出範圍回錯誤。
#[test]
fn screening_graham_counts_as_filter() {
    let mut params = screening_params();
    params.max_pe_pb_product = Some(22.5);
    assert!(validate_screening_params(&params).is_ok());
    params.max_pe_pb_product = Some(1001.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("max_pe_pb_product 必須介於 0 至 1000")
    );

    let mut params = screening_params();
    params.min_current_ratio = Some(2.0);
    assert!(validate_screening_params(&params).is_ok());
    params.min_current_ratio = Some(101.0);
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("min_current_ratio 必須介於 0 至 100")
    );

    let mut params = screening_params();
    params.sort_by = Some("pe_pb_product".to_owned());
    params.sort_order = Some("asc".to_owned());
    assert!(validate_screening_params(&params).is_ok());
}

/// 每檔附乘積與流動比率；依乘積升冪為 GRD、GRA、GRB，GRC 虧損排最後；`max_pe_pb_product=22.5` 留 GRD、GRA，
/// `min_current_ratio=2` 只留 GRA（GRC 過期、GRD 流動負債為 0 都是 `null`）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_and_sorts_by_graham_criteria() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_and_sorts_by_graham_criteria：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM balance_sheet WHERE stock_symbol LIKE '79979GR_'",
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979GR_'"#,
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979GR_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, pe, pb, year, quarter, current_assets, current_liabilities) in ROWS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '葛拉漢', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "PriceEarningRatio", "price-to-book_ratio")
               VALUES ('1988-06-10', $1, 10, $2::numeric, $3::numeric)"#,
        )
        .bind(symbol)
        .bind(pe)
        .bind(pb)
        .execute(pool)
        .await
        .expect("種入日線");
        sqlx::query(
            "INSERT INTO balance_sheet
                 (stock_symbol, fiscal_year, period_type, quarter, source, current_assets, current_liabilities)
             VALUES ($1, $2, 'single', $3, 'yahoo', $4, $5)",
        )
        .bind(symbol)
        .bind(year)
        .bind(quarter)
        .bind(current_assets)
        .bind(current_liabilities)
        .execute(pool)
        .await
        .expect("種入資產負債表");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 6, 10).expect("固定日期合法");
    let screen = |max_product: Option<f64>, min_current_ratio: Option<f64>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.max_pe_pb_product = max_product;
        params.min_current_ratio = min_current_ratio;
        params.limit = Some(50);
        params.sort_by = Some("pe_pb_product".to_owned());
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
                    .is_some_and(|symbol| symbol.starts_with("79979GR"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"].clone(),
                    stock["pe_pb_product"].clone(),
                    stock["current_ratio"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let all = screen(None, None).await;
    let cheap = screen(Some(22.5), None).await;
    let liquid = screen(None, Some(2.0)).await;
    cleanup().await;

    assert_eq!(
        all,
        [
            (json!("79979GRD"), json!(8.0), json!(null)),
            (json!("79979GRA"), json!(15.0), json!(3.0)),
            (json!("79979GRB"), json!(40.0), json!(1.5)),
            (json!("79979GRC"), json!(null), json!(null)),
        ]
    );
    assert_eq!(
        cheap,
        [
            (json!("79979GRD"), json!(8.0), json!(null)),
            (json!("79979GRA"), json!(15.0), json!(3.0)),
        ]
    );
    assert_eq!(liquid, [(json!("79979GRA"), json!(15.0), json!(3.0))]);
}
