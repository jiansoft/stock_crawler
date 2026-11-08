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
