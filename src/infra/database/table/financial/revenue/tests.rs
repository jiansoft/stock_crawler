//! 月營收資料表的測試。

//use crate::internal::database::table::revenue;

use std::str::FromStr;

use chrono::{Datelike, FixedOffset, Local, NaiveDate, TimeDelta, TimeZone};
use rust_decimal::Decimal;

//use chrono::{Datelike, Local, NaiveDate};
use crate::infra::database::table::revenue::{fetch_last_two_month, rebuild_revenue_last_date};

#[tokio::test]
async fn test_date() {
    // 取得本月的第一天
    let now = Local::now();
    let first_day_of_month = NaiveDate::from_ymd_opt(now.year(), now.month(), 1);

    // 取得上個月的第一天
    let last_month = if now.month() == 1 {
        NaiveDate::from_ymd_opt(now.year() - 1, 12, 1)
    } else {
        NaiveDate::from_ymd_opt(now.year(), now.month() - 1, 1)
    };

    // 取得二個月前的第一天
    let two_month_ago = if now.month() <= 2 {
        NaiveDate::from_ymd_opt(now.year() - 1, now.month() + 10, 1)
    } else {
        NaiveDate::from_ymd_opt(now.year(), now.month() - 2, 1)
    };

    println!("This month's first day: {:?}", first_day_of_month);
    println!("Last month's first day: {:?}", last_month);
    println!("Two months ago first day: {:?}", two_month_ago);

    let now_first_day = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();

    let last_month = now_first_day - TimeDelta::try_minutes(1).unwrap();
    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let last_month_timezone = timezone.from_local_datetime(&last_month).unwrap();
    let two_month_ago_first_day = NaiveDate::from_ymd_opt(last_month.year(), last_month.month(), 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let two_month_ago = two_month_ago_first_day - TimeDelta::try_minutes(1).unwrap();
    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let two_month_ago_timezone = timezone.from_local_datetime(&two_month_ago).unwrap();

    println!("This month's first day: {:?}", now_first_day);
    println!("Last month's last day: {:?}", last_month_timezone);
    println!("Two months ago last day: {:?}", two_month_ago_timezone);
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
    let now = Local
        .with_ymd_and_hms(1999, 3, 15, 9, 0, 0)
        .single()
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
