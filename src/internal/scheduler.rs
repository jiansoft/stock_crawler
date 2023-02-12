use crate::{
    internal::crawler::international_securities_identification_number,
    internal::crawler::taiwan_capitalization_weighted_stock_index,
    internal::free_dns,
    internal::crawler::StockMarket
};
use clokwerk::{AsyncScheduler, Job, TimeUnits};
use std::time::Duration;
use crate::internal::crawler::suspend_listing;

/// 啟動排程
pub async fn start() {
    let mut scheduler = AsyncScheduler::new();
    //每日下午三點更新台股收盤指數
    scheduler.every(1.day()).at("15:00:00").run(|| async {
        taiwan_capitalization_weighted_stock_index::visit().await;
    });

    //每日五點更新台股台股國際證券識別碼
    scheduler.every(1.day()).at("5:00:00").run(|| async {
        international_securities_identification_number::visit(StockMarket::StockExchange).await;
        suspend_listing::visit().await;
        international_securities_identification_number::visit(StockMarket::OverTheCounter).await;
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
