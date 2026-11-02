//! 持股近期股利入帳 endpoint 的資料庫語意整合測試。

use chrono::{Days, NaiveDate};

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

const MEMBER: i64 = 979_798;

async fn cleanup_payouts(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM dividend_record_detail_more WHERE stock_ownership_details_serial IN \
         (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
        "DELETE FROM dividend_record_detail WHERE stock_ownership_details_serial IN \
         (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
        "DELETE FROM stock_ownership_details WHERE member_id = $1",
    ] {
        let _ = sqlx::query(sql).bind(MEMBER).execute(pool).await;
    }
    cleanup(pool).await;
}

/// 種一筆股利，回傳 serial；日期以 `None` 表示 `-`。
async fn seed_dividend(
    pool: &sqlx::PgPool,
    quarter: &str,
    cash: i32,
    stock: i32,
    ex: NaiveDate,
    payable: Option<NaiveDate>,
) -> i64 {
    let text = |date: Option<NaiveDate>| date.map_or_else(|| "-".to_owned(), |d| d.to_string());
    sqlx::query_scalar(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2099, 2099, $2, $3, $4, $3 + $4, $5, $6, $7, $8) RETURNING serial"#)
        .bind(SYMBOL)
        .bind(quarter)
        .bind(rust_decimal::Decimal::from(cash))
        .bind(rust_decimal::Decimal::from(stock))
        .bind(ex.to_string())
        .bind(if stock > 0 { ex.to_string() } else { "-".to_owned() })
        .bind(text(payable))
        .bind(if stock > 0 { text(payable) } else { "-".to_owned() })
        .fetch_one(pool)
        .await
        .expect("種入 dividend")
}

/// 已除權息的取已領股利明細（除權息後賣出的仍列入）；尚未除權息的以目前持股推算並標 estimated，
/// 同一天發放的現金與股票股利合為一筆；已除權息但沒有明細的、發放日在期間外的都不列；參數超出
/// 範圍回 422。日期只能由今天推算；列屬於假代號與不存在的會員，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_upcoming_payouts_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("portfolio_upcoming_payouts_endpoint_db_semantics").await else {
        return;
    };
    cleanup_payouts(pool).await;

    let path = format!("/api/v1/portfolio/upcoming-payouts?member_id={MEMBER}&days=30");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["items"], serde_json::json!([]), "沒有持股");

    seed(pool, &[SEED_STOCK]).await;
    let today = chrono::Local::now().date_naive();
    let day = |offset: i64| {
        if offset >= 0 {
            today + Days::new(offset.unsigned_abs())
        } else {
            today - Days::new(offset.unsigned_abs())
        }
    };
    let holding = |shares: i64, sold: bool| async move {
        sqlx::query_scalar::<_, i64>("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES ($1, $2, $3, -1, $4) RETURNING serial")
            .bind(MEMBER)
            .bind(SYMBOL)
            .bind(shares)
            .bind(sold)
            .fetch_one(pool)
            .await
            .expect("種入持股")
    };
    let active = holding(1000, false).await;
    let sold = holding(500, true).await;

    // 已除息、5 天後發放：賣掉的 500 股有明細。
    let recorded = seed_dividend(pool, "Q1", 2, 0, day(-10), Some(day(5))).await;
    let detail: i64 = sqlx::query_scalar("INSERT INTO dividend_record_detail (stock_ownership_details_serial, year, cash, total) VALUES ($1, 2099, 1000, 1000) RETURNING serial")
        .bind(sold)
        .fetch_one(pool)
        .await
        .expect("種入已領股利");
    sqlx::query("INSERT INTO dividend_record_detail_more (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial, cash, total) VALUES ($1, $2, $3, 1000, 1000)")
        .bind(sold)
        .bind(detail)
        .bind(recorded)
        .execute(pool)
        .await
        .expect("種入已領股利明細");
    // 3 天後除權息、20 天後同一天發放現金與股票：以目前 1000 股推算。
    seed_dividend(pool, "Q2", 1, 1, day(3), Some(day(20))).await;
    // 已除息但沒有明細：不推算。
    seed_dividend(pool, "Q3", 4, 0, day(-5), Some(day(7))).await;
    // 發放日在 30 天以外。
    seed_dividend(pool, "Q4", 3, 0, day(10), Some(day(40))).await;

    let (status, json) = get(&path).await;
    let (bad_member, _) = get("/api/v1/portfolio/upcoming-payouts?member_id=-1").await;
    let (zero_days, _) = get("/api/v1/portfolio/upcoming-payouts?days=0").await;
    let (too_many_days, _) = get("/api/v1/portfolio/upcoming-payouts?days=91").await;
    cleanup_payouts(pool).await;
    let _ = active;

    assert_eq!(status, StatusCode::OK, "{json}");
    let items = json["items"].as_array().expect("items 陣列");
    let summary: Vec<(String, String, i64, f64, i64, bool)> = items
        .iter()
        .map(|item| {
            (
                item["payable_date"].as_str().unwrap_or_default().to_owned(),
                item["quarter"].as_str().unwrap_or_default().to_owned(),
                item["shares"].as_i64().unwrap_or_default(),
                item["cash_amount"].as_f64().unwrap_or_default(),
                item["stock_shares"].as_i64().unwrap_or_default(),
                item["estimated"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (day(5).to_string(), "Q1".to_owned(), 500, 1000.0, 0, false),
            (
                day(20).to_string(),
                "Q2".to_owned(),
                1000,
                1000.0,
                100,
                true
            ),
        ],
        "{json}"
    );
    assert_eq!(items[1]["ex_date"], day(3).to_string());
    assert_eq!(json["total_cash_amount"], 2000.0);
    assert_eq!(bad_member, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(zero_days, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(too_many_days, StatusCode::UNPROCESSABLE_ENTITY);
}
