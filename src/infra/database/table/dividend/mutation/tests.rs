//! 股利資料表寫入操作的測試。

use chrono::Local;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::Row;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 upsert");
    let mut e = Dividend::new();
    e.security_code = String::from("79979");
    e.year = 2023;
    e.year_of_dividend = 2023;
    e.quarter = String::from("H1");
    e.ex_dividend_date1 = "尚未公布".to_string();
    e.ex_dividend_date2 = "尚未公布".to_string();
    e.payable_date1 = "尚未公布".to_string();
    e.payable_date2 = "尚未公布".to_string();
    e.created_time = Local::now();
    e.updated_time = Local::now();
    e.cash_dividend = dec!(1);
    e.stock_dividend = dec!(2);
    e.sum = dec!(3);
    e.capital_reserve_cash_dividend = dec!(0.5);
    e.earnings_cash_dividend = dec!(0.5);
    e.capital_reserve_stock_dividend = dec!(1);
    e.earnings_stock_dividend = dec!(1);
    e.payout_ratio = dec!(99);
    e.payout_ratio_cash = dec!(45);
    e.payout_ratio_stock = dec!(44);

    match e.upsert().await {
        Ok(result) => {
            tracing::debug!("{:?} {:?} ", result, e);
        }
        Err(why) => {
            tracing::debug!("Failed to upsert because {:?} ", why);
        }
    }

    // 假代號的測試資料必須清除，否則會殘留在資料庫裡影響其他查詢。
    let _ = sqlx::query("DELETE FROM dividend WHERE security_code = $1 AND year = $2")
        .bind(&e.security_code)
        .bind(e.year)
        .execute(database::get_connection())
        .await;

    tracing::debug!("結束 upsert");
}

/// 驗證 `upsert` 在主鍵衝突時會同步更新除權息日與股利發放日。
///
/// 此測試會實際寫入 `dividend` 表。先用測試股票代碼寫入 `尚未公布` 日期，再以同一主鍵
/// upsert 正式日期，最後查回確認四個日期欄位都已被覆蓋。測試結束會刪除測試股票代碼資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert_updates_dividend_dates_on_conflict() {
    dotenvy::dotenv().ok();

    let security_code = "__TEST_UPSERT_DATE__";
    let year = 2099;
    let cleanup_sql = "DELETE FROM dividend WHERE security_code = $1;";

    sqlx::query(cleanup_sql)
        .bind(security_code)
        .execute(database::get_connection())
        .await
        .expect("cleanup dividend test rows before test");

    let mut unpublished = Dividend::new();
    unpublished.security_code = security_code.to_string();
    unpublished.year = year;
    unpublished.year_of_dividend = year - 1;
    unpublished.cash_dividend = dec!(42);
    unpublished.sum = dec!(42);
    unpublished.ex_dividend_date1 = "尚未公布".to_string();
    unpublished.ex_dividend_date2 = "-".to_string();
    unpublished.payable_date1 = "尚未公布".to_string();
    unpublished.payable_date2 = "-".to_string();
    unpublished
        .upsert()
        .await
        .expect("insert unpublished dividend row");

    let mut published = unpublished.clone();
    published.cash_dividend = dec!(43);
    published.sum = dec!(43);
    published.ex_dividend_date1 = "2099-06-26".to_string();
    published.ex_dividend_date2 = "2099-06-27".to_string();
    published.payable_date1 = "2099-07-17".to_string();
    published.payable_date2 = "2099-07-18".to_string();
    published
        .upsert()
        .await
        .expect("update published dividend row");

    let row = sqlx::query(
        r#"
SELECT cash_dividend,
       "ex-dividend_date1",
       "ex-dividend_date2",
       payable_date1,
       payable_date2
FROM dividend
WHERE security_code = $1 AND year = $2 AND quarter = '';
"#,
    )
    .bind(security_code)
    .bind(year)
    .fetch_one(database::get_connection())
    .await
    .expect("fetch updated dividend row");

    assert_eq!(row.get::<Decimal, _>("cash_dividend"), dec!(43));
    assert_eq!(row.get::<String, _>("ex-dividend_date1"), "2099-06-26");
    assert_eq!(row.get::<String, _>("ex-dividend_date2"), "2099-06-27");
    assert_eq!(row.get::<String, _>("payable_date1"), "2099-07-17");
    assert_eq!(row.get::<String, _>("payable_date2"), "2099-07-18");

    sqlx::query(cleanup_sql)
        .bind(security_code)
        .execute(database::get_connection())
        .await
        .expect("cleanup dividend test rows after test");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert_annual_total_dividend_operates_database() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 upsert_annual_total_dividend");

    // 此函式 SQL 只使用股票代號與發放年度兩個參數；測試只補齊必要參數並確認 SQL 可執行。
    let mut annual_total_seed = Dividend::new();
    annual_total_seed.security_code = "5306".to_string();
    annual_total_seed.year = 2026;

    annual_total_seed
        .upsert_annual_total_dividend()
        .await
        .expect("upsert annual total dividend");

    tracing::debug!("結束 upsert_annual_total_dividend");
}
