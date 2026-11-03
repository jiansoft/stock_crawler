//! 持股年度股利入帳 endpoint 的資料庫語意整合測試。

use chrono::{Datelike, Days, Months, NaiveDate};

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

const MEMBER: i64 = 979_799;

async fn cleanup_income(pool: &sqlx::PgPool) {
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

/// 種一筆股利（現金與股票同一天除權息、發放），回傳 serial。
async fn seed_dividend(
    pool: &sqlx::PgPool,
    quarter: &str,
    cash: i32,
    stock: i32,
    ex: NaiveDate,
    payable: NaiveDate,
) -> i64 {
    let stock_date = |date: NaiveDate| {
        if stock > 0 {
            date.to_string()
        } else {
            "-".to_owned()
        }
    };
    sqlx::query_scalar(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2099, 2099, $2, $3, $4, $3 + $4, $5, $6, $7, $8) RETURNING serial"#)
        .bind(SYMBOL)
        .bind(quarter)
        .bind(rust_decimal::Decimal::from(cash))
        .bind(rust_decimal::Decimal::from(stock))
        .bind(ex.to_string())
        .bind(stock_date(ex))
        .bind(payable.to_string())
        .bind(stock_date(payable))
        .fetch_one(pool)
        .await
        .expect("種入 dividend")
}

/// 一筆持股與它在某筆股利的已領明細（現金 `cash` 元）。
async fn seed_recorded(pool: &sqlx::PgPool, shares: i64, dividend: i64, cash: i32) {
    let holding: i64 = sqlx::query_scalar("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES ($1, $2, $3, -1, true) RETURNING serial")
        .bind(MEMBER)
        .bind(SYMBOL)
        .bind(shares)
        .fetch_one(pool)
        .await
        .expect("種入持股");
    let detail: i64 = sqlx::query_scalar("INSERT INTO dividend_record_detail (stock_ownership_details_serial, year, cash, total) VALUES ($1, 2099, $2, $2) RETURNING serial")
        .bind(holding)
        .bind(rust_decimal::Decimal::from(cash))
        .fetch_one(pool)
        .await
        .expect("種入已領股利");
    sqlx::query("INSERT INTO dividend_record_detail_more (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial, cash, total) VALUES ($1, $2, $3, $4, $4)")
        .bind(holding)
        .bind(detail)
        .bind(dividend)
        .bind(rust_decimal::Decimal::from(cash))
        .execute(pool)
        .await
        .expect("種入已領股利明細");
}

/// 今天入帳的已領明細算已入帳，明天除權息的以目前持股推算（跨年時不算今年），去年同日的入帳算
/// 去年同月與同期；參數超出範圍回 422。日期只能由今天推算；列屬於假代號與不存在的會員，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_dividend_income_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("portfolio_dividend_income_endpoint_db_semantics").await else {
        return;
    };
    cleanup_income(pool).await;
    let today = chrono::Local::now().date_naive();
    let path = format!(
        "/api/v1/portfolio/dividend-income?member_id={MEMBER}&year={}",
        today.year()
    );

    let (status, empty) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["total_cash_amount"], 0.0, "沒有持股");
    assert_eq!(empty["months"].as_array().map(Vec::len), Some(12));

    seed(pool, &[SEED_STOCK]).await;
    // 10 天前除息、今天發放：賣掉的 500 股有明細 1000 元。
    let received = seed_dividend(pool, "Q1", 2, 0, today - Days::new(10), today).await;
    seed_recorded(pool, 500, received, 1000).await;
    // 明天除權息、同一天發放現金與股票：以目前 1000 股推算。
    let tomorrow = today + Days::new(1);
    seed_dividend(pool, "Q2", 1, 1, tomorrow, tomorrow).await;
    sqlx::query("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES ($1, $2, 1000, -1, false)")
        .bind(MEMBER)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入目前持股");
    // 去年的今天入帳 900 元。
    let last_year_day = today - Months::new(12);
    let last_year = seed_dividend(
        pool,
        "Q3",
        3,
        0,
        last_year_day - Days::new(10),
        last_year_day,
    )
    .await;
    seed_recorded(pool, 300, last_year, 900).await;

    let (status, json) = get(&path).await;
    let (bad_member, _) = get("/api/v1/portfolio/dividend-income?member_id=-1").await;
    let (bad_year, _) = get("/api/v1/portfolio/dividend-income?year=1999").await;
    cleanup_income(pool).await;

    assert_eq!(status, StatusCode::OK, "{json}");
    let (estimated_cash, estimated_shares) = if tomorrow.year() == today.year() {
        (1000.0, 100)
    } else {
        (0.0, 0)
    };
    assert_eq!(json["year"], today.year());
    assert_eq!(json["as_of"], today.to_string());
    assert_eq!(json["received_cash_amount"], 1000.0, "{json}");
    assert_eq!(json["estimated_cash_amount"], estimated_cash, "{json}");
    assert_eq!(json["total_cash_amount"], 1000.0 + estimated_cash, "{json}");
    assert_eq!(json["total_stock_shares"], estimated_shares, "{json}");
    assert_eq!(json["last_year_cash_amount"], 900.0, "{json}");
    assert_eq!(json["last_year_same_period_cash_amount"], 900.0, "{json}");
    let month = &json["months"][today.month0() as usize];
    assert_eq!(month["month"], today.month());
    assert_eq!(month["received_cash_amount"], 1000.0, "{json}");
    assert_eq!(month["last_year_cash_amount"], 900.0, "{json}");
    assert_eq!(bad_member, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad_year, StatusCode::UNPROCESSABLE_ENTITY);
}
