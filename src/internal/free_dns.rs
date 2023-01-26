use crate::{config, logging};
use clokwerk::{AsyncScheduler, TimeUnits};
use std::time::Duration;

/// 向ddns服務更新目前的IP
pub fn update() {
    let mut scheduler = AsyncScheduler::new();
    scheduler.every(60.seconds()).run(|| async {
        let url = format!(
            "{}{}?{}",
            config::SETTINGS.afraid.url,
            config::SETTINGS.afraid.path,
            config::DEFAULT.afraid.token
        );

        match reqwest::get(url).await {
            Ok(res) => match res.text().await {
                Ok(t) => {
                    logging::info_file_async(t);
                }
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                }
            },
            Err(why) => {
                logging::error_file_async(format!("{:?}", why));
            }
        }
    });

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}
