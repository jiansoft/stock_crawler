use crate::{
    internal::calculation, internal::crawler::international_securities_identification_number,
    internal::crawler::revenue, internal::crawler::suspend_listing,
    internal::crawler::taiwan_capitalization_weighted_stock_index, internal::crawler::StockMarket,
    internal::free_dns,
};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate};
use clokwerk::{AsyncScheduler, Interval, Job, TimeUnits};
use std::time::Duration;

/// 啟動排程
pub async fn start() {
    let mut scheduler = AsyncScheduler::new();

    //每日五點更新台股台股國際證券識別碼
    scheduler
        .every(Interval::Days(1))
        .at("5:00:00")
        .run(|| async {
            international_securities_identification_number::visit(StockMarket::Listed).await;
            suspend_listing::visit().await;
            international_securities_identification_number::visit(StockMarket::OverTheCounter)
                .await;
            international_securities_identification_number::visit(StockMarket::Emerging).await;
            let now = Local::now();
            let naive_datetime = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let last_month = naive_datetime - chrono::Duration::minutes(1);
            let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
            let last_month_timezone = DateTime::<FixedOffset>::from_local(last_month, timezone);
            revenue::visit(last_month_timezone).await;
        });

    //每日下午三點更新台股收盤指數
    scheduler
        .every(Interval::Days(1))
        .at("15:00:00")
        .run(|| async {
            taiwan_capitalization_weighted_stock_index::visit().await;
        });

    scheduler
        .every(Interval::Days(1))
        .at("15:00:00")
        .run(|| async {
            let now = Local::now();
            calculation::dividend_record::calculate(now.year()).await;
        });

    scheduler.every(60.seconds()).run(|| async {
        free_dns::update().await;
    });

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}
