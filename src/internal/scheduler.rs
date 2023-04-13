use crate::{
    internal::{
        backfill, backfill::delisted_company, backfill::taiwan_capitalization_weighted_stock_index,
        bot, crawler, crawler::quotes, crawler::revenue, reminder,
    },
    logging,
};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate};
use clokwerk::{AsyncScheduler, Interval, Job, TimeUnits};
use std::time::Duration;

/// 啟動排程
pub async fn start() {
    let mut scheduler = AsyncScheduler::new();

    scheduler
        .every(Interval::Days(1))
        .at("01:00:00")
        .run(|| async {

            //將未有上季度財報的股票，到雅虎財經下載後回寫到 financial_statement 表
            match backfill::financial_statement::execute().await {
                Ok(_) => {
                    logging::info_file_async(
                        "backfill::financial_statement::execute executed successfully.".to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to backfill::financial_statement::execute because {:?}",
                        why
                    ));
                }
            }

            //更新興櫃股票的每股淨值
            match backfill::net_asset_value_per_share::emerging::execute().await {
                Ok(_) => {
                    logging::info_file_async(
                        "backfill::net_asset_value_per_share::emerging::execute executed successfully."
                            .to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to backfill::net_asset_value_per_share::emerging::execute because {:?}",
                        why
                    ));
                }
            }
        });

    scheduler
        .every(Interval::Days(1))
        .at("03:00:00")
        .run(|| async {
            //從yahoo取得每股淨值數據，將未下市但每股淨值為零的股票更新其數據
            match backfill::net_asset_value_per_share::zero_value::execute().await {
                Ok(_) => {
                    logging::info_file_async(
                        "backfill::net_asset_value_per_share::zero_value::execute executed successfully."
                            .to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to backfill::net_asset_value_per_share::zero_value::execute because {:?}",
                        why
                    ));
                }
            }
        });

    //每日五點更新台股台股國際證券識別碼
    scheduler
        .every(Interval::Days(1))
        .at("5:00:00")
        .run(|| async {
            //取得台股國際證券識別碼
            match backfill::international_securities_identification_number::execute().await {
                Ok(_) => {
                    logging::info_file_async(
                        "international_securities_identification_number::execute executed successfully."
                            .to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to international_securities_identification_number::execute because {:?}",
                        why
                    ));
                }
            }

            //更新下市的股票
            match delisted_company::execute().await {
                Ok(_) => {
                    logging::info_file_async(
                        "delisted_company::visit executed successfully."
                            .to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to delisted_company::visit because {:?}",
                        why
                    ));
                }
            }

            let now = Local::now();
            let naive_datetime = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let last_month = naive_datetime - chrono::Duration::minutes(1);
            let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
            let last_month_timezone = DateTime::<FixedOffset>::from_local(last_month, timezone);
            //取得台股上月的營收
            revenue::visit(last_month_timezone).await;
        });

    //每日上午八點
    scheduler
        .every(Interval::Days(1))
        .at("08:00:00")
        .run(|| async {
            let today: NaiveDate = Local::now().date_naive();
            //提醒本日除權息的股票
            reminder::ex_dividend::execute(today).await;
        });

    //每日下午三點
    scheduler
        .every(Interval::Days(1))
        .at("15:00:00")
        .run(|| async {
            //更新台股收盤指數
            match taiwan_capitalization_weighted_stock_index::execute().await {
                Ok(_) => {
                    logging::info_file_async(
                        "taiwan_capitalization_weighted_stock_index::execute executed successfully."
                            .to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to taiwan_capitalization_weighted_stock_index::execute because {:?}",
                        why
                    ));
                }
            }

            //取得上市收盤報價數據
            match quotes::listed::visit(Local::now()).await {
                Ok(_) => {
                    logging::info_file_async(
                        "quotes::listed::visit executed successfully.".to_string(),
                    );
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to quotes::listed::visit because {:?}",
                        why
                    ));
                }
            }
        });

    /* scheduler
    .every(Interval::Days(1))
    .at("15:00:00")
    .run(|| async {
        let now = Local::now();
        //計算股利領取
        calculation::dividend_record::calculate(now.year()).await;
    });*/

    scheduler.every(60.seconds()).run(|| async {
        crawler::free_dns::update().await;
    });

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let _ = bot::telegram::send_to_allowed("StockCrawler-Rust 已啟動").await;
}
