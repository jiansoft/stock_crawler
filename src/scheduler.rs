use std::{env, future::Future};

use anyhow::{Error, Result};
use tokio::task;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};

use crate::event::ddns;
use crate::{
    backfill::{
        delisted_company, dividend, financial_statement, isin, net_asset_value_per_share,
        qualified_foreign_institutional_investor, revenue, stock_weight,
    },
    bot, event, logging,
};

/// 啟動排程
pub async fn start(sched: &JobScheduler) -> Result<()> {
    run_cron(sched).await?;

    let s = sched.clone();

    task::spawn(async move {
        if let Err(why) = event::trace::stock_price::execute().await {
            logging::error_file_async(format!("{:?}", why));
        }

        // 09:00 提醒本日已達高低標的股票有那些
        if let Ok(j) = create_job("0 0 1 * * *", event::trace::stock_price::execute) {
            if let Err(why) = s.add(j).await {
                logging::error_file_async(format!("{:?}", why));
            }
        }
    });

    let msg = format!(
        "StockCrawler 已啟動\r\nRust OS/Arch: {}/{}\r\n",
        env::consts::OS,
        env::consts::ARCH
    );

    bot::telegram::send(&msg).await
}

async fn run_cron(sched: &JobScheduler) -> std::result::Result<(), JobSchedulerError> {
    //let sched = JobScheduler::new().await?;
    //                 sec  min   hour   day of month   month   day of week   year
    //let expression = "0   30   9,12,15     1,15       May-Aug  Mon,Wed,Fri  2018/2";
    // UTC 時間

    let jobs = vec![
        // 01:00 更新興櫃股票的每股淨值
        create_job("0 0 17 * * *", net_asset_value_per_share::emerging::execute),
        // 02:30 更新盈餘分配率
        create_job("0 30 18 * * *", dividend::payout_ratio::execute),
        // 03:00 更新台股季度財報
        create_job("0 0 19 * * *", event::taiwan_stock::quarter_eps::execute),
        // 04:00 更新台股季度財報
        create_job("0 0 20 * * *", financial_statement::quarter::execute),
        // 05:00 更新台股年度財報(僅有eps 等少數欄位的資料)
        create_job("0 0 21 * * *", event::taiwan_stock::annual_eps::execute),
        // 05:00 更新台股年度財報
        create_job("0 0 21 * * *", financial_statement::annual::execute),
        // 05:00 從yahoo取得每股淨值數據，將未下市但每股淨值為零的股票更新其數據
        create_job(
            "0 0 21 * * *",
            net_asset_value_per_share::zero_value::execute,
        ),
        // 05:00 取得台股的營收
        create_job("0 0 21 * * *", revenue::execute),
        // 05:00 更新台股國際證券識別碼
        create_job("0 0 21 * * *", isin::execute),
        // 05:00 更新下市的股票
        create_job("0 0 21 * * *", delisted_company::execute),
        // 05:00 更新股票權值佔比
        create_job("0 0 21 * * *", stock_weight::execute),
        // 08:00 提醒本日除權息的股票
        create_job("0 0 0 * * *", event::taiwan_stock::ex_dividend::execute),
        // 08:00 提醒本日發放股利的股票(只通知自已有的股票)
        create_job("0 0 0 * * *", event::taiwan_stock::payable_date::execute),
        // 08:00 提醒本日開始公開申購的股票
        create_job("0 0 0 * * *", || async {
            event::taiwan_stock::public::execute().await
            //Ok(())
        }),
        // 15:00 取得收盤報價數據
        create_job("0 0 7 * * *", event::taiwan_stock::closing::execute),
        // 21:00 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
        create_job("0 0 13 * * *", dividend::execute),
        // 22:00 外資持股狀態
        create_job(
            "0 0 14 * * *",
            qualified_foreign_institutional_investor::execute,
        ),
        // 每分鐘更新一次ddns的ip
        create_job("0 * * * * *", ddns::refresh),
    ];

    for job in jobs.into_iter().flatten() {
        sched.add(job).await?;
    }

    sched.start().await
}

pub trait Scheduler {
    fn is_weekend(&self) -> bool;
}

fn create_job<F, Fut>(cron_expr: &'static str, task: F) -> Result<Job>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send,
{
    Ok(Job::new_async(cron_expr, move |_uuid, _l| {
        let task = task.clone();
        Box::pin(async move {
            if let Err(why) = task().await {
                logging::error_file_async(format!(
                    "Failed to execute task({}) because {:?}",
                    cron_expr, why
                ));
            }
        })
    })?)
}

#[cfg(test)]
mod tests {
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    async fn run() -> Result<()> {
        let sched = JobScheduler::new().await?;
        let every_minute = Job::new_async("* * * * * *", |_uuid, _l| {
            Box::pin(async move {
                println!("_uuid {:?} now: {:?}", _uuid, chrono::Local::now());
                dbg!("_uuid {:?} now: {:?}", _uuid, chrono::Local::now());
                logging::debug_file_async(format!(
                    "_uuid {:?} now: {:?}",
                    _uuid,
                    chrono::Local::now()
                ));
            })
        })?;
        sched.add(every_minute).await?;

        sched.start().await?;

        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_split() {
        dotenv::dotenv().ok();
        run().await.expect("TODO: panic message");
        //sleep(Duration::from_secs(240)).await;
        //loop {}
        //println!("split: {:?}, elapsed time: {:?}", result, end);
    }
}
