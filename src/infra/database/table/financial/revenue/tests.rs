//! 月營收資料表的測試。

//use crate::internal::database::table::revenue;

use crate::core::util::datetime::TaipeiDateTime;
use jiff::ToSpan;
use std::str::FromStr;

use rust_decimal::Decimal;

use crate::infra::database::table::revenue::{fetch_last_two_month, rebuild_revenue_last_date};

/// 本月 1 日、上個月 1 日與兩個月前 1 日；前一天分別落在上個月與兩個月前。
#[test]
fn test_date() {
    let now = TaipeiDateTime::now();
    let first_day_of_month = now.date().first_of_month();
    let last_month = (first_day_of_month - 1.day()).first_of_month();
    let two_month_ago = (last_month - 1.day()).first_of_month();

    println!("This month's first day: {first_day_of_month}");
    println!("Last month's first day: {last_month}");
    println!("Two months ago first day: {two_month_ago}");
    assert_eq!(first_day_of_month.day(), 1);
    assert_eq!(last_month + 1.month(), first_day_of_month);
    assert_eq!(two_month_ago + 2.months(), first_day_of_month);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_last_two_month() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 fetch_last_two_month");

    let m = Decimal::from_str("0.00".replace([',', ' '], "").as_str()).unwrap_or_else(|err| {
        eprintln!("Failed to parse 'compared_with_last_month' field: {}", err);
        Default::default()
    });
    println!("m={}", m);
    match fetch_last_two_month().await {
        Ok(result) => {
            for e in result {
                tracing::info!("{:?} ", e);
            }
        }
        Err(why) => {
            tracing::error!("because {:?}", why);
        }
    }
    /* if let Ok(result) = r {
        for e in result {
            tracing::info!("{:#?} ", e);
        }
    }*/
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_rebuild_revenue_last_date() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 test_rebuild_revenue_last_date");
    match rebuild_revenue_last_date().await {
        Ok(result) => {
            tracing::info!("rebuild_revenue_last_date:{:?} ", result.rows_affected());
        }
        Err(why) => {
            tracing::error!("Failed to rebuild_revenue_last_date because {:?}", why);
        }
    }
}

/// 以臨時資料庫測「前兩個月」的查詢與資料列轉換：1999-03 查到 199902、199901，不含 199812。
/// 假代號 79979RV，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn fetch_last_two_months_before_reads_previous_two_months() {
    use crate::infra::database;
    use crate::infra::database::table::revenue::fetch_last_two_months_before;

    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_last_two_months_before_reads_previous_two_months：無資料庫連接");
        return;
    }
    let symbol = "79979RV";
    let cleanup = || async {
        sqlx::query(r#"DELETE FROM "Revenue" WHERE stock_symbol = $1"#)
            .bind(symbol)
            .execute(pool)
            .await
            .expect("清除測試營收");
    };
    cleanup().await;
    for (date, monthly) in [(199_812_i64, 100), (199_901, 200), (199_902, 300)] {
        sqlx::query(
            r#"INSERT INTO "Revenue" (stock_symbol, "Date", "Monthly", "LastMonth", "LastYearThisMonth",
                   "MonthlyAccumulated", "LastYearMonthlyAccumulated", "ComparedWithLastMonth",
                   "ComparedWithLastYearSameMonth", "AccumulatedComparedWithLastYear",
                   avg_price, lowest_price, highest_price)
               VALUES ($1, $2, $3, 0, 0, $3, 0, 0, 0, 0, 10, 9, 11)"#,
        )
        .bind(symbol)
        .bind(date)
        .bind(Decimal::from(monthly))
        .execute(pool)
        .await
        .expect("插入測試營收");
    }
    let now = TaipeiDateTime::from_datetime(jiff::civil::date(1999, 3, 15).at(9, 0, 0, 0))
        .expect("合法時間");

    let revenues = fetch_last_two_months_before(now).await;
    cleanup().await;

    let mut ours: Vec<(i64, Decimal)> = revenues
        .expect("查詢成功")
        .into_iter()
        .filter(|revenue| revenue.stock_symbol == symbol)
        .map(|revenue| (revenue.date, revenue.monthly))
        .collect();
    ours.sort();
    assert_eq!(
        ours,
        vec![(199_901, Decimal::from(200)), (199_902, Decimal::from(300))]
    );
}
