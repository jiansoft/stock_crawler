use std::{env, future::Future, time::Instant};

use anyhow::{Context, Error, Result};
use chrono::FixedOffset;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    app::backfill::{
        delisted_company, dividend, etf, financial_statement, isin, net_asset_value_per_share,
        qualified_foreign_institutional_investor, revenue, stock_weight,
    },
    app::event,
    // 通知一律走 core::alert 抽象介面（port）：app 層不 import
    // interfaces::bot（傳輸層細節），實際的 Telegram adapter 由 main 啟動時註冊。
    // 這樣 use case 可以在單元測試中注入假的 sink 驗證，也能日後替換通知管道。
    core::{alert, declare, util::text},
};

/// 啟動排程
pub async fn start(sched: &JobScheduler) -> Result<()> {
    let timer = Instant::now();
    tracing::info!("scheduler start begin: run_cron");
    let run_cron_timer = Instant::now();
    run_cron(sched).await.context("Failed to run cron jobs")?;
    tracing::info!(
        "scheduler start done: run_cron elapsed={:?}",
        run_cron_timer.elapsed()
    );

    //若在開盤埘間重啟服務定時任務會無法觸發，所以在啟動時要先執行股價追踪的任務，執行完後再設定一次定時任務
    if declare::StockExchange::TWSE.is_open() {
        tracing::info!("scheduler start begin: opening trace::stock_price");
        let opening_trace_timer = Instant::now();
        if let Err(why) = event::trace::stock_price::execute().await {
            let err_msg = format!("{:?}", why);
            tracing::error!("{}", &err_msg);
            alert::send_alert("開盤股價追蹤初始化失敗", &err_msg).await;
        }
        tracing::info!(
            "scheduler start done: opening trace::stock_price elapsed={:?}",
            opening_trace_timer.elapsed()
        );
    }

    let msg = format!(
        "StockCrawler 已啟動\r\nRust OS/Arch: {}/{}\r\n",
        // OS/Arch 是動態內容，照規則先做 MarkdownV2 跳脫再組進訊息。
        text::escape_markdown_v2(env::consts::OS),
        text::escape_markdown_v2(env::consts::ARCH)
    );

    tracing::info!("scheduler start begin: telegram notify");
    let telegram_timer = Instant::now();
    alert::send_message(&msg).await;
    tracing::info!(
        "scheduler start done: telegram notify elapsed={:?}",
        telegram_timer.elapsed()
    );
    tracing::info!("scheduler start done: total elapsed={:?}", timer.elapsed());

    Ok(())
}

/// 註冊所有 cron 任務並啟動排程器。
async fn run_cron(sched: &JobScheduler) -> Result<()> {
    let timer = Instant::now();
    //let sched = JobScheduler::new().await?;
    //                 sec  min   hour   day of month   month   day of week   year
    //let expression = "0   30   9,12,15     1,15       May-Aug  Mon,Wed,Fri  2018/2";
    // 台北時間（UTC+8）：create_job 內以 Job::new_async_tz 搭配 FixedOffset(+8)
    // 解讀 cron 表達式，因此以下註解標示的時間皆為台北時間。

    let jobs = vec![
        // 01:00 更新興櫃股票的每股淨值
        create_job(
            "0 0 1 * * *",
            "更新興櫃股票每股淨值",
            net_asset_value_per_share::emerging::execute,
        ),
        // 02:30 更新盈餘分配率
        create_job(
            "0 30 2 * * *",
            "更新盈餘分配率",
            dividend::payout_ratio::execute,
        ),
        // 03:00 更新台股季度財報
        create_job(
            "0 0 3 * * *",
            "更新台股季度財報",
            event::taiwan_stock::quarter_eps::execute,
        ),
        // 04:00 更新台股季度財報(ROE、ROA為零的數據)
        create_job(
            "0 0 4 * * *",
            "補齊季報零淨值之 ROE/ROA 數據",
            financial_statement::quarter::execute,
        ),
        // 05:00 更新台股年度財報(僅有eps 等少數欄位的資料)
        create_job(
            "0 0 5 * * *",
            "更新年度財報基本 EPS",
            event::taiwan_stock::annual_eps::execute,
        ),
        // 05:05 更新台股年度財報
        create_job(
            "0 5 5 * * *",
            "更新完整年度財報",
            financial_statement::annual::execute,
        ),
        // 05:10 從yahoo取得每股淨值數據，將未下市但每股淨值為零的股票更新其數據
        create_job(
            "0 10 5 * * *",
            "更新每股淨值(補 Yahoo 零淨值數據)",
            net_asset_value_per_share::zero_value::execute,
        ),
        // 05:15 取得台股的營收
        create_job("0 15 5 * * *", "取得台股月度營收", revenue::execute),
        // 05:20 更新台股國際證券識別碼
        create_job("0 20 5 * * *", "更新台股 ISIN 識別碼", isin::execute),
        // 05:25 更新下市的股票
        create_job(
            "0 25 5 * * *",
            "更新下市股票清單",
            delisted_company::execute,
        ),
        // 05:30 更新台股 ETF 資訊
        create_job("0 30 5 * * *", "更新台股 ETF 資訊", etf::execute),
        // 08:00 提醒本日除權息與明日預計除權息的股票
        create_job(
            "0 0 8 * * *",
            "提醒本日與明日除權息股票",
            event::taiwan_stock::ex_dividend::execute,
        ),
        // 08:02 提醒本日發放股利的股票(只通知自已有的股票)
        create_job(
            "0 2 8 * * *",
            "提醒本日持股股利發放",
            event::taiwan_stock::payable_date::execute,
        ),
        // 08:04 提醒本日開始公開申購的股票
        create_job(
            "0 4 8 * * *",
            "提醒本日公開申購股票",
            event::taiwan_stock::public::execute,
        ),
        // 09:00 更新股票權值佔比
        create_job("0 0 9 * * *", "更新股票加權權值佔比", stock_weight::execute),
        // 09:02 提醒本日已達高低標的股票有那些
        create_job(
            "0 2 9 * * *",
            "監控並提醒股價達高低標",
            event::trace::stock_price::execute,
        ),
        // 15:00 取得收盤報價數據
        create_job(
            "0 0 15 * * *",
            "取得每日收盤行情與報價",
            event::taiwan_stock::closing::execute,
        ),
        // 21:00 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
        create_job("0 0 21 * * *", "補齊缺失之年度配息數據", dividend::execute),
        // 22:00 外資持股狀態
        create_job(
            "0 0 22 * * *",
            "更新外資持股比例與狀態",
            qualified_foreign_institutional_investor::execute,
        ),
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
    tracing::info!(
        "scheduler run_cron done: jobs={}, elapsed={:?}",
        job_count,
        timer.elapsed()
    );

    Ok(())
}

/// 排程輔助介面。
pub trait Scheduler {
    /// 判斷目前時間是否為週末。
    fn is_weekend(&self) -> bool;
}

/// 將非同步工作包裝成 `tokio_cron_scheduler::Job`，並自動記錄執行時間。
///
/// 每次任務註冊與觸發時會：
/// 1. 於啟動註冊階段記錄排程任務名稱與運作週期。
/// 2. 觸發時記錄 `task.begin` 事件（含 cron 表達式與任務名稱作為結構化欄位）。
/// 3. 執行任務並計時。
/// 4. 記錄 `task.done` 事件（含 `elapsed_ms`）或 `task.failed` 事件（含錯誤訊息）。
///
/// 結構化欄位（`task`、`name`、`elapsed_ms`）透過 F1 `FieldCollector` 同步送到 Seq，
/// 讓 ops 可直接在 Seq 查詢特定任務的執行時間趨勢。
fn create_job<F, Fut>(cron_expr: &'static str, name: &'static str, task: F) -> Result<Job>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send,
{
    tracing::info!("註冊排程任務: [{}] 運作週期: '{}'", name, cron_expr);
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    Ok(Job::new_async_tz(cron_expr, tz, move |_uuid, _l| {
        let task = task.clone();
        Box::pin(async move {
            // 排程一旦「真正開始執行」才登記為 active，主程式關機時會等待此 use case 離開。
            // guard 必須放在 async block 內，不能放在 create_job 階段；後者只是在啟動時
            // 註冊排程，若那時就 +1，counter 會在整個服務生命週期永遠無法歸零。
            let _operation_guard = crate::core::shutdown::BACKGROUND_OPERATIONS.begin();
            tracing::info!(task = cron_expr, name = name, "task.begin");
            let t = Instant::now();
            match task().await {
                Ok(()) => {
                    tracing::info!(
                        task = cron_expr,
                        name = name,
                        elapsed_ms = t.elapsed().as_millis() as u64,
                        "task.done"
                    );
                }
                Err(why) => {
                    let err_msg = format!("{:?}", why);
                    tracing::error!(
                        task = cron_expr,
                        name = name,
                        elapsed_ms = t.elapsed().as_millis() as u64,
                        error = %err_msg,
                        "task.failed"
                    );
                    alert::send_alert(&format!("排程任務 [{}] 執行失敗", name), &err_msg).await;
                }
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
                tracing::debug!("_uuid {:?} now: {:?}", _uuid, chrono::Local::now());
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
        dotenvy::dotenv().ok();
        run().await.expect("TODO: panic message");
        //sleep(Duration::from_secs(240)).await;
        //loop {}
        //println!("split: {:?}, elapsed time: {:?}", result, end);
    }
}
