//! 條件選股整合測試：千張大戶持股比例變化。

use super::*;

/// 以固定歷史查詢日 1988-03-05（週六）執行選股：只用日期早於查詢日的週，查詢日當天的週不會提前用到；
/// 1 週變化 03-04 對 02-26 為 +1、2 週變化 03-04 對 02-19 為 +1.5。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_major_change() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_major_change：無資料庫連接");
        return;
    }
    const SYMBOL: &str = "79979SM";
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM holder_distribution WHERE stock_symbol = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
        }
    };
    cleanup().await;
    for sql in [
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, '大戶選股', $1, 2, 1)"#,
        // 03-05 與查詢日同一天，比例跌到 40：集保週六才公布，不應用到。
        r#"INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders)
           VALUES ('1988-02-19', $1, 10, 50, 100), ('1988-02-26', $1, 10, 50.5, 100),
                  ('1988-03-04', $1, 11, 51.5, 100), ('1988-03-05', $1, 9, 40, 100)"#,
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("種入大戶選股資料");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 3, 5).expect("固定日期合法");
    let listed = |minimum: f64, weeks: Option<u8>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.min_major_change = Some(minimum);
        params.major_change_weeks = weeks;
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
            .any(|stock| stock["stock_symbol"] == SYMBOL)
    };

    let one_week = listed(1.0, Some(1)).await;
    let one_week_too_high = listed(1.1, Some(1)).await;
    let two_weeks = listed(1.5, Some(2)).await;
    let two_weeks_too_high = listed(1.6, Some(2)).await;
    let three_weeks = listed(-100.0, Some(3)).await;
    cleanup().await;
    assert!(one_week, "1 週變化 +1，應出現");
    assert!(!one_week_too_high, "1 週變化只有 +1");
    assert!(two_weeks, "2 週變化 +1.5，應出現");
    assert!(!two_weeks_too_high, "2 週變化只有 +1.5");
    assert!(!three_weeks, "往前第 3 週沒有資料，不應出現");
}
