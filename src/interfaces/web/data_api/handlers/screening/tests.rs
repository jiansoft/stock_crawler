//! 條件選股參數驗證、排序白名單與新鮮度公式的 deterministic tests，
//! 以及以真實 PostgreSQL 驗證執行計畫的 P0-4 整合測試。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::params::{
    analytical_date_is_fresh, f_score_year_is_fresh, financial_period_is_fresh,
    revenue_month_is_fresh, screen_order_by, validate_screening_params,
};
use super::sql::SCREEN_STOCKS_SQL;
use crate::infra::database;
use crate::interfaces::web::data_api::dto::StockScreeningParams;

/// 建立只覆寫測試重點欄位的 screen 預設參數。
fn screening_params() -> StockScreeningParams {
    StockScreeningParams {
        market: None,
        industry_id: None,
        valuation_band: None,
        min_revenue_yoy_percent: None,
        min_eps: None,
        min_roe_percent: None,
        min_dividend_yield_percent: None,
        min_f_score: None,
        min_gross_margin_percent: None,
        max_debt_ratio_percent: None,
        positive_free_cash_flow: None,
        sort_by: None,
        sort_order: None,
        limit: None,
    }
}

/// `all` 不縮小市場集合，不能單獨通過；上市／上櫃或任何一個實質條件
/// 都可成立，固定預設排序與 limit 也應在驗證後補齊。
#[test]
fn screening_requires_a_meaningful_filter() {
    assert_eq!(
        validate_screening_params(&screening_params()).err(),
        Some("至少需要一個篩選條件")
    );
    let mut explicit_all = screening_params();
    explicit_all.market = Some("all".to_owned());
    assert!(validate_screening_params(&explicit_all).is_err());

    let mut twse = screening_params();
    twse.market = Some("twse".to_owned());
    let validated = validate_screening_params(&twse).expect("twse 本身是有效條件");
    assert_eq!(validated.market_id, 2);
    assert_eq!(validated.limit, 20);
    assert_eq!(validated.order_by, "ORDER BY stock_symbol ASC");

    let mut industry = screening_params();
    industry.industry_id = Some(24);
    assert!(validate_screening_params(&industry).is_ok());
    let mut minimum = screening_params();
    minimum.min_eps = Some(0.0);
    assert!(validate_screening_params(&minimum).is_ok());

    // 財務體質條件各自都算有效篩選；positive_free_cash_flow=false 等於沒有條件。
    let mut f_score = screening_params();
    f_score.min_f_score = Some(0);
    assert!(validate_screening_params(&f_score).is_ok());
    let mut debt = screening_params();
    debt.max_debt_ratio_percent = Some(50.0);
    assert!(validate_screening_params(&debt).is_ok());
    let mut cash_flow = screening_params();
    cash_flow.positive_free_cash_flow = Some(true);
    assert!(validate_screening_params(&cash_flow).is_ok());
    cash_flow.positive_free_cash_flow = Some(false);
    assert_eq!(
        validate_screening_params(&cash_flow).err(),
        Some("至少需要一個篩選條件")
    );
}

/// 財務體質條件的範圍：F 分數 0–9、毛利率上限 100%、負債比 0–1000%。
#[test]
fn screening_health_boundaries_are_deterministic() {
    let mut f_score = screening_params();
    f_score.min_f_score = Some(9);
    assert!(validate_screening_params(&f_score).is_ok());
    f_score.min_f_score = Some(10);
    assert_eq!(
        validate_screening_params(&f_score).err(),
        Some("min_f_score 必須介於 0 至 9")
    );
    for (value, ok) in [
        (100.0, true),
        (-10_000.0, true),
        (100.01, false),
        (f64::NAN, false),
    ] {
        let mut params = screening_params();
        params.min_gross_margin_percent = Some(value);
        assert_eq!(validate_screening_params(&params).is_ok(), ok, "{value}");
    }
    for (value, ok) in [
        (0.0, true),
        (1_000.0, true),
        (-0.01, false),
        (1_000.01, false),
    ] {
        let mut params = screening_params();
        params.max_debt_ratio_percent = Some(value);
        assert_eq!(validate_screening_params(&params).is_ok(), ok, "{value}");
    }
}

/// 九個排序欄位乘上兩種方向必須完整映射，指標降冪也要明確
/// `NULLS LAST`；不在白名單的文字必須在接觸 SQL 前遭拒。
#[test]
fn screening_sort_has_eighteen_static_branches() {
    let fields = [
        "stock_symbol",
        "revenue_yoy",
        "eps",
        "roe",
        "dividend_yield",
        "valuation_percentage",
        "f_score",
        "gross_margin",
        "debt_ratio",
    ];
    for field in fields {
        for order in ["asc", "desc"] {
            let sql = screen_order_by(field, order).expect("白名單組合應存在");
            assert!(sql.starts_with("ORDER BY "));
            if field != "stock_symbol" {
                assert!(sql.contains("NULLS LAST"));
                assert!(sql.ends_with("stock_symbol ASC"));
            }
        }
    }
    assert!(screen_order_by("closing_price; DROP TABLE stocks", "asc").is_err());
    assert!(screen_order_by("eps", "sideways").is_err());
}

/// 數值範圍包含契約兩端，超界、NaN 與無限大都必須回 422；這可避免
/// 非有限浮點數進入 Decimal 轉換或 PostgreSQL 比較。
#[test]
fn screening_numeric_boundaries_are_deterministic() {
    for value in [-100.0, 10_000.0] {
        let mut params = screening_params();
        params.min_revenue_yoy_percent = Some(value);
        assert!(validate_screening_params(&params).is_ok());
    }
    for value in [-100.01, 10_000.01, f64::NAN, f64::INFINITY] {
        let mut params = screening_params();
        params.min_revenue_yoy_percent = Some(value);
        assert!(validate_screening_params(&params).is_err());
    }
    let mut yield_at_zero = screening_params();
    yield_at_zero.min_dividend_yield_percent = Some(0.0);
    assert!(validate_screening_params(&yield_at_zero).is_ok());
    let mut negative_yield = screening_params();
    negative_yield.min_dividend_yield_percent = Some(-0.01);
    assert!(validate_screening_params(&negative_yield).is_err());
}

/// 新鮮度邊界使用月份／季度序號處理跨年，並以「差三月／兩季／三十日」
/// 為仍有效的最後一天；未來日期一律不視為新鮮。
#[test]
fn screening_freshness_boundaries_cross_years() {
    let query_date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    assert!(revenue_month_is_fresh(query_date, 202510));
    assert!(!revenue_month_is_fresh(query_date, 202509));
    assert!(!revenue_month_is_fresh(query_date, 202602));

    assert!(financial_period_is_fresh(query_date, 2025, "Q3"));
    assert!(!financial_period_is_fresh(query_date, 2025, "Q2"));
    assert!(!financial_period_is_fresh(query_date, 2026, "Q2"));
    assert!(!financial_period_is_fresh(query_date, 2025, ""));

    // F 分數：2026 年查詢時 2024、2025 年報仍有效（年報 3 月底前才公布）。
    assert!(f_score_year_is_fresh(query_date, 2025));
    assert!(f_score_year_is_fresh(query_date, 2024));
    assert!(!f_score_year_is_fresh(query_date, 2023));
    assert!(!f_score_year_is_fresh(query_date, 2026));

    let thirty_days_ago = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let thirty_one_days_ago = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
    assert!(analytical_date_is_fresh(query_date, thirty_days_ago));
    assert!(!analytical_date_is_fresh(query_date, thirty_one_days_ago));
    assert!(!analytical_date_is_fresh(
        query_date,
        NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
    ));
}

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
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {SCREEN_STOCKS_SQL}\nORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC\nLIMIT $13"
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
        .bind(20_i64)
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
