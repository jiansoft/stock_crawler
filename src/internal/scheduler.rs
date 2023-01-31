use crate::{internal::crawler::taiwan_capitalization_weighted_stock_index, internal::free_dns};
use clokwerk::{AsyncScheduler, Job, TimeUnits};
use std::time::Duration;

/// 啟動排程
pub async fn start() {
    let mut scheduler = AsyncScheduler::new();
    //每日下午三點下載台股收盤指數
    scheduler.every(1.day()).at("15:00:00").run(|| async {
        taiwan_capitalization_weighted_stock_index::visit().await;
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
