//! 連資料庫跑一次除權息事件：從派發器的預設處理器進入，隔年發放的股利也要寫進已領股利（H28）。

use std::sync::Arc;

use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::PgPool;

use crate::app::event::handlers::EventDispatcher;
use crate::app::event::handlers::debounce::TelegramDebouncer;
use crate::domain::events::DomainEvent;
use crate::infra::database;

const SYMBOL: &str = "79979EX";
const MEMBER: i64 = 979_796;

async fn cleanup(pool: &PgPool) {
    for sql in [
        "DELETE FROM dividend_record_detail_more WHERE stock_ownership_details_serial IN \
         (SELECT serial FROM stock_ownership_details WHERE security_code = $1)",
        "DELETE FROM dividend_record_detail WHERE stock_ownership_details_serial IN \
         (SELECT serial FROM stock_ownership_details WHERE security_code = $1)",
        "DELETE FROM stock_ownership_details WHERE security_code = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("清除測試資料");
    }
}

async fn seed(pool: &PgPool) {
    sqlx::query(r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing") VALUES ($1, '除息事件測試', $1, 2, 1, false)"#)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入股票");
    // 2024-12-10 除息、2025-01-09 發放：year 是 2025（發放年度）。
    sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2025, 2024, 'Q3', 4.5, 0, 4.5, '2024-12-10', '-', '2025-01-09', '-')"#)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入股利");
    sqlx::query(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold, created_time, date) \
         VALUES ($1, $2, 1000, -1, false, '2024-06-03 09:00:00+08', '2024-06-03')",
    )
    .bind(MEMBER)
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入持股");
}

fn ex_dividend_event(date: NaiveDate, next_trading_date: NaiveDate) -> DomainEvent {
    DomainEvent::ExDividendReminderTriggered {
        date,
        next_trading_date,
        occurred_at: Local::now(),
    }
}

/// 前一個交易日只有「預計除權息」公告、不重算；除息當天重算持股股利，12 月除息、
/// 隔年發放的股利記在下一個發放年度。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn ex_dividend_event_records_next_year_payout() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 ex_dividend_event_records_next_year_payout：無資料庫連接");
        return;
    }
    cleanup(pool).await;
    seed(pool).await;
    let day = |d: u32| NaiveDate::from_ymd_opt(2024, 12, d).expect("合法日期");
    let debouncer = Arc::new(TelegramDebouncer::new());
    let records = || async {
        sqlx::query_as::<_, (i32, Decimal)>(
            "SELECT r.year, r.cash FROM dividend_record_detail r \
             JOIN stock_ownership_details s ON s.serial = r.stock_ownership_details_serial \
             WHERE s.security_code = $1 ORDER BY r.year",
        )
        .bind(SYMBOL)
        .fetch_all(pool)
        .await
        .expect("查已領股利")
    };

    let eve = EventDispatcher::default_handle_event(
        ex_dividend_event(day(9), day(10)),
        debouncer.clone(),
    )
    .await;
    let after_eve = records().await;
    let on_day =
        EventDispatcher::default_handle_event(ex_dividend_event(day(10), day(11)), debouncer).await;
    let after_day = records().await;
    let items: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dividend_record_detail_more m \
         JOIN stock_ownership_details s ON s.serial = m.stock_ownership_details_serial \
         WHERE s.security_code = $1",
    )
    .bind(SYMBOL)
    .fetch_one(pool)
    .await
    .expect("查已領股利明細");
    cleanup(pool).await;

    eve.expect("前一日的事件應成功");
    on_day.expect("除息當天的事件應成功");
    assert!(
        after_eve.is_empty(),
        "前一日只發預計公告，不重算：{after_eve:?}"
    );
    assert_eq!(
        after_day,
        vec![(2025, dec!(4500))],
        "4.5 元 × 1,000 股記在發放年度 2025"
    );
    assert_eq!(items, 1, "明細一筆");
}
