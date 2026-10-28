//! 以真實 PostgreSQL 驗證條件選股核心查詢的執行計畫（EXPLAIN ANALYZE）。

use super::*;

/// P0-4：以實際 PostgreSQL 對 Phase 2/3 核心查詢執行 EXPLAIN ANALYZE。
///
/// 測試輸出文字 plan 供人工記錄 latency 與 scan 類型，並固定條件選股
/// 四個「每股最新一期」子查詢使用既有索引；沒有資料庫時明確跳過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn phase2_phase3_query_plans_use_existing_indexes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 P0-4 EXPLAIN：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    // EXPLAIN 只需要型別正確的查詢參數，不需要依賴資料庫預先匯入股票。
    // 使用固定代號可讓全新 CI 資料庫也能驗證執行計畫，且不會寫入測試資料。
    let symbol = "2330";

    // SQL 與 production 使用相同 WHERE、JOIN、排序及 LIMIT；EXPLAIN 不寫資料。
    let valuation: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT security_code, date, closing_price FROM estimate WHERE security_code = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30)) ORDER BY date DESC LIMIT 1"#)
        .bind(symbol).bind(Option::<NaiveDate>::None).fetch_all(pool).await.expect("valuation EXPLAIN");
    let breadth: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) WITH endpoint AS (SELECT MAX(date) AS date FROM daily_stock_price_stats WHERE stock_exchange_market_id = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30))) SELECT s.date FROM daily_stock_price_stats s CROSS JOIN endpoint e WHERE s.stock_exchange_market_id = $1 AND s.date <= e.date ORDER BY s.date DESC LIMIT $3"#)
        .bind(0_i32).bind(Option::<NaiveDate>::None).bind(20_i64).fetch_all(pool).await.expect("breadth EXPLAIN");
    let ranking: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT y.security_code FROM yield_rank y JOIN stocks s ON s.stock_symbol = y.security_code JOIN "DailyQuotes" q ON q."Serial" = y.daily_quotes_serial JOIN dividend d ON d.serial = y.dividend_serial WHERE y.date = (SELECT MAX(date) FROM yield_rank) AND s.stock_exchange_market_id IN (2,4) ORDER BY y.yield DESC, y.security_code ASC LIMIT 20"#)
        .fetch_all(pool).await.expect("ranking EXPLAIN");
    let screen_sql = format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {SCREEN_STOCKS_SQL}\nORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC\nLIMIT $24"
    );
    let screen: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(screen_sql.as_str()))
        .bind(0_i32)
        .bind(Option::<i32>::None)
        .bind(chrono::Local::now().date_naive())
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<&str>::None)
        .bind(Option::<i16>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(false)
        .bind(Option::<i16>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Vec::<String>::new())
        .bind(Vec::<i16>::new())
        .bind(Option::<i16>::None)
        .bind(Vec::<String>::new())
        .bind(Vec::<i16>::new())
        .bind(Vec::<i16>::new())
        .bind(Option::<i16>::None)
        .bind(Option::<i16>::None)
        .bind(20_i64)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(false)
        .bind(false)
        .bind(Option::<Vec<i32>>::None)
        .bind(Option::<Decimal>::None)
        .bind(Option::<Decimal>::None)
        .bind(Vec::<String>::new())
        .bind(Vec::<Decimal>::new())
        .bind(Option::<Decimal>::None)
        .fetch_all(pool)
        .await
        .expect("screen EXPLAIN");

    for (name, plan) in [
        ("valuation", valuation),
        ("breadth", breadth),
        ("yield-ranking", ranking),
        ("screen", screen),
    ] {
        assert!(!plan.is_empty(), "{name} 應回傳 plan");
        println!("\n===== {name} =====\n{}", plan.join("\n"));
        if name == "screen" {
            assert!(
                plan.join("\n")
                    .contains("yield_rank-security_code-date-desc-idx"),
                "screen 每股最新殖利率必須走複合索引"
            );
        }
    }
}
