use std::{env, future::Future, time::Instant};

use anyhow::{Context, Error, Result};
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    backfill::{
        delisted_company, dividend, financial_statement, isin, net_asset_value_per_share,
        qualified_foreign_institutional_investor, revenue, stock_weight,
    },
    bot::{self, telegram::Telegram},
    declare, event, logging,
};

/// 啟動排程
pub async fn start(sched: &JobScheduler) -> Result<()> {
    let timer = Instant::now();
    logging::info_file_async("scheduler start begin: run_cron".to_string());
    let run_cron_timer = Instant::now();
    run_cron(sched).await.context("Failed to run cron jobs")?;
    logging::info_file_async(format!(
        "scheduler start done: run_cron elapsed={:?}",
        run_cron_timer.elapsed()
    ));

    //若在開盤埘間重啟服務定時任務會無法觸發，所以在啟動時要先執行股價追踪的任務，執行完後再設定一次定時任務
    if declare::StockExchange::TWSE.is_open() {
        logging::info_file_async("scheduler start begin: opening trace::stock_price".to_string());
        let opening_trace_timer = Instant::now();
        if let Err(why) = event::trace::stock_price::execute().await {
            logging::error_file_async(format!("{:?}", why));
        }
        logging::info_file_async(format!(
            "scheduler start done: opening trace::stock_price elapsed={:?}",
            opening_trace_timer.elapsed()
        ));
    }

    let msg = format!(
        "StockCrawler 已啟動\r\nRust OS/Arch: {}/{}\r\n",
        Telegram::escape_markdown_v2(env::consts::OS),
        Telegram::escape_markdown_v2(env::consts::ARCH)
    );

    logging::info_file_async("scheduler start begin: telegram notify".to_string());
    let telegram_timer = Instant::now();
    bot::telegram::send(&msg).await;
    logging::info_file_async(format!(
        "scheduler start done: telegram notify elapsed={:?}",
        telegram_timer.elapsed()
    ));
    logging::info_file_async(format!(
        "scheduler start done: total elapsed={:?}",
        timer.elapsed()
    ));

    Ok(())
}

/// 註冊所有 cron 任務並啟動排程器。
async fn run_cron(sched: &JobScheduler) -> Result<()> {
    let timer = Instant::now();
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
        // 04:00 更新台股季度財報(ROE、ROA為零的數據)
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
        // 08:00 提醒本日除權息的股票
        create_job("0 0 0 * * *", event::taiwan_stock::ex_dividend::execute),
        // 08:00 提醒本日發放股利的股票(只通知自已有的股票)
        create_job("0 0 0 * * *", event::taiwan_stock::payable_date::execute),
        // 08:00 提醒本日開始公開申購的股票
        create_job("0 0 0 * * *", || async {
            event::taiwan_stock::public::execute().await
            //Ok(())
        }),
        // 09:00 更新股票權值佔比
        create_job("0 0 1 * * *", stock_weight::execute),
        // 09:00 提醒本日已達高低標的股票有那些
        create_job("0 0 1 * * *", event::trace::stock_price::execute),
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
        //create_job("0 * * * * *", ddns::refresh),
    ];

    let mut job_count = 0usize;
    for job in jobs.into_iter().flatten() {
        sched
            .add(job)
            .await
            .context("Failed to add job to scheduler")?;
        job_count += 1;
    }

    sched.start().await.context("Failed to start scheduler")?;
    logging::info_file_async(format!(
        "scheduler run_cron done: jobs={}, elapsed={:?}",
        job_count,
        timer.elapsed()
    ));

    Ok(())
}

/// 排程輔助介面。
pub trait Scheduler {
    /// 判斷目前時間是否為週末。
    fn is_weekend(&self) -> bool;
}

/// 將非同步工作包裝成 `tokio_cron_scheduler::Job`。
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

    /// 建立測試用排程器並註冊每秒任務。
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

    /// 手動執行排程 smoke test。
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
