//! 股利預估快照：已公告與已除息（已領明細）的配息都寫入，同一天重跑整批替換。
//!
//! 快照日固定 2026-04-01，會員 979_791、代號 79978A，測完刪除。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::infra::database;

const MEMBER: i64 = 979_791;
const SYMBOL: &str = "79978A";

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

async fn cleanup(pool: &sqlx::PgPool, snapshot_date: NaiveDate) {
    let _ = sqlx::query("DELETE FROM dividend_forecast_snapshot WHERE snapshot_date = $1")
        .bind(snapshot_date)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM dividend_record_detail_more WHERE stock_ownership_details_serial IN (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
    )
    .bind(MEMBER)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
        .bind(MEMBER)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM dividend WHERE security_code = $1")
        .bind(SYMBOL)
        .execute(pool)
        .await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn record_writes_announced_and_entitled_items_and_replaces_rerun() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!(
            "跳過 record_writes_announced_and_entitled_items_and_replaces_rerun：無資料庫連接"
        );
        return;
    }
    let today = date(2026, 4, 1);
    cleanup(pool, today).await;

    let serial: i64 = sqlx::query_scalar(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost) VALUES ($1, $2, 1000, -50000) RETURNING serial",
    )
    .bind(MEMBER)
    .bind(SYMBOL)
    .fetch_one(pool)
    .await
    .expect("種入持股");
    for (quarter, cash, ex, payable) in [
        // 已除息、未發放：金額取已領明細的 800 元（不是 1 元 × 1,000 股）。
        ("Q4", 1, "2026-03-25", "2026-04-20"),
        // 尚未除息：依目前 1,000 股。
        ("Q1", 2, "2026-06-10", "2026-07-10"),
    ] {
        sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2026, 2026, $2, $3, 0, $3, $4, '-', $5, '-')"#)
            .bind(SYMBOL)
            .bind(quarter)
            .bind(Decimal::from(cash))
            .bind(ex)
            .bind(payable)
            .execute(pool)
            .await
            .expect("種入 dividend");
    }
    sqlx::query(
        "INSERT INTO dividend_record_detail_more (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial, cash, total) SELECT $1, 0, serial, 800, 800 FROM dividend WHERE security_code = $2 AND quarter = 'Q4'",
    )
    .bind(serial)
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入已領明細");

    let first = record(today).await.expect("第一次快照");
    let second = record(today).await.expect("同一天重跑");
    let rows: Vec<(NaiveDate, i64, Decimal, String, bool)> = sqlx::query_as(
        "SELECT ex_date, shares, amount, basis, entitled FROM dividend_forecast_snapshot WHERE snapshot_date = $1 AND member_id = $2 ORDER BY ex_date",
    )
    .bind(today)
    .bind(MEMBER)
    .fetch_all(pool)
    .await
    .expect("查詢快照");
    cleanup(pool, today).await;

    assert_eq!(first, second, "重跑應整批替換、筆數相同");
    assert_eq!(
        rows,
        [
            (
                date(2026, 3, 25),
                1_000,
                dec!(800),
                "announced".to_owned(),
                true
            ),
            (
                date(2026, 6, 10),
                1_000,
                dec!(2000),
                "announced".to_owned(),
                false
            ),
        ]
    );
}
