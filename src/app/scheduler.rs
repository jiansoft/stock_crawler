use std::{
    env,
    future::Future,
    time::{Duration, Instant},
};

use anyhow::{Context, Error, Result};
use chrono::FixedOffset;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    app::backfill::{
        capital_reduction, chip, delisted_company, dividend, etf, financial_report,
        financial_statement, isin, net_asset_value_per_share, official_ratio,
        qualified_foreign_institutional_investor, revenue, stock_weight,
    },
    app::calculation,
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
        // 05:32 回補交易所公告的減資事件
        // 上櫃的 revivt 公告只涵蓋當週、無法查歷史，因此必須每天抓才不會漏。
        // 必須排在 05:40 的 CAGR 之前：減資造成的價格跳動要先登錄成公司行動，
        // 當日的報酬率才不會把它誤算成真實漲跌。
        create_job(
            "0 32 5 * * *",
            "回補交易所公告的減資事件",
            capital_reduction::execute,
        ),
        // 05:35 掃描交易所除權息公告，補齊漏抓的股利事件
        // 必須排在 05:40 的 CAGR 之前，讓當日新補進來的股利能立刻被納入計算
        create_job(
            "0 35 5 * * *",
            "掃描除權息公告補齊漏抓事件",
            dividend::scan_announcements,
        ),
        // 05:37 以交易所除權除息計算結果核對近 45 天的股利日期與金額
        // Yahoo 會留著延後除息前的舊日期；必須排在 05:40 的 CAGR 之前，含息報酬才會用到正確的除息日
        create_job(
            "0 37 5 * * *",
            "核對交易所除權息結果",
            dividend::reconcile_ex_right_results,
        ),
        // 05:40 計算各期間年化報酬率(CAGR)
        // 必須排在 21:00 年度配息回補與 05:00~05:30 各項回補之後，
        // 否則當日結果會少算前一晚才補進來的股利
        create_job(
            "0 40 5 * * *",
            "計算各期間年化報酬率(CAGR)",
            calculation::cagr::execute_scheduled,
        ),
        // 06:00 採集 Yahoo 三大財務報表（損益表、資產負債表、現金流量表）
        // 每輪最多 400 檔、約 45 分鐘；7 天 Redis 旗標讓每檔約每週重抓一次。
        // Yahoo 失敗（404 除外）的股票改從 BigGo 財經備援。
        // 避開 21:00 的 Yahoo 股利採集，兩者不會同時對 Yahoo 發請求。
        create_job(
            "0 0 6 * * *",
            "採集 Yahoo 三大財務報表",
            financial_report::execute,
        ),
        // 07:10 以三大財務報表重算全市場財務體質（F 分數、毛利率、負債比、近四季自由現金流），
        // 排在 06:00 採集（約 35～45 分鐘）之後；條件選股以這張表篩選。
        create_job(
            "0 10 7 * * *",
            "重算財務體質評分",
            calculation::financial_health::execute,
        ),
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
        // 15:30（週一至週五）大盤趨勢變化通知：加權指數當天站上或跌破 60／240 日均線、
        // 均線排列改變才發；15:00 收盤事件已寫入加權指數，最新一筆不是今天（休市）就不發。
        create_job(
            "0 30 15 * * Mon-Fri",
            "通知大盤趨勢變化",
            event::taiwan_stock::market_regime_alert::execute,
        ),
        // 17:00、19:00（週一至週五）檢查當日收盤資料：上市或上櫃有成交的檔數明顯偏少
        // （來源尚未公布、暫時失敗）就重跑收盤匯總並告警，避免缺口被「缺漏補齊」的零量列藏起來。
        // 行情完整後再檢查技術訊號與未來報酬，沒寫入就只重算缺的那一步並告警；大盤指數沒寫入也重抓。
        create_job(
            "0 0 17,19 * * Mon-Fri",
            "檢查收盤資料完整性",
            event::taiwan_stock::closing::ensure_complete,
        ),
        // 17:20（週一至週五）通知今天填息的持股：17:00 已確認收盤資料完整（不完整會重跑），
        // 以「填息日是今天」判斷，同一次除權息只會通知一次；休市日沒有今天的收盤，不會發送。
        create_job(
            "0 20 17 * * Mon-Fri",
            "通知持股填息",
            event::taiwan_stock::dividend_fill_alert::execute,
        ),
        // 20:30 通知持股近期法說會的摘要、展望與 Q&A 重點（BigGo 整理）
        // 法說會多在下午舉行，BigGo 通常當天或隔天產生摘要；摘要未產生的場次之後的排程再看。
        create_job(
            "0 30 20 * * *",
            "通知持股法說會摘要",
            event::taiwan_stock::earnings_call::execute,
        ),
        // 20:40（週一至週五）通知持股主力進出：主力買賣超佔成交量達門檻的股票與前幾名分點
        // 富邦證券（MoneyDJ）的分點資料於盤後更新；頁面日期不是今天的股票略過，休市日不會發訊息。
        create_job(
            "0 40 20 * * Mon-Fri",
            "通知持股主力進出",
            event::taiwan_stock::broker_flow::execute,
        ),
        // 20:45（每月 10–28 日）通知持股董監質押變動
        // 董監事持股明細約每月 18 日出表；與 Redis 保存的前一期比較，同月份只通知一次。
        create_job(
            "0 45 20 10-28 * *",
            "通知持股董監質押變動",
            event::taiwan_stock::insider_pledge::execute,
        ),
        // 10:20（週六）每週市場摘要：產業 RS 中位數與一週變化、外資／投信近 5 日產業淨買賣、
        // 持股 RS 一週變化；週五的籌碼 21:50 入庫、報酬收盤後回填，週六早上資料已齊。
        create_job(
            "0 20 10 * * Sat",
            "每週市場摘要",
            event::taiwan_stock::weekly_market::execute,
        ),
        // 10:30（週六）持股千張大戶比例週報：集保股權分散表每週六公布上週五的資料，
        // 與 Redis 中上週的快照比較；同一份資料只通知一次。
        create_job(
            "0 30 10 * * Sat",
            "通知持股千張大戶週報",
            event::taiwan_stock::holder_distribution::execute,
        ),
        // 10:50（週六）持股估值週報：本益比、股價淨值比位於自己近 5 年 ≥ 第 90 或 ≤ 第 10 百分位的持股；
        // 沒有就不發。
        create_job(
            "0 50 10 * * Sat",
            "通知持股估值週報",
            event::taiwan_stock::valuation_alert::execute,
        ),
        // 11:00（週六）資料健康週報：近 7 天的交易日完整性、均線、漲跌幅、衍生資料新鮮度、
        // 月營收與股利佔位列；這類問題來源不會報錯，只能定期量。
        create_job(
            "0 0 11 * * Sat",
            "資料健康週報",
            event::taiwan_stock::data_health::execute,
        ),
        // 19:15（週一至週五）通知持股的重要技術訊號與相對強度轉折；排在 19:00 收盤完整性檢查之後，
        // 若有重跑收盤匯總，訊號也已重新計算。
        create_job(
            "0 15 19 * * Mon-Fri",
            "通知持股技術訊號",
            event::taiwan_stock::technical_alert::execute,
        ),
        // 21:00 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
        create_job("0 0 21 * * *", "補齊缺失之年度配息數據", dividend::execute),
        // 21:40（週一至週五）通知持股三大法人與融資融券的明顯動作
        // 融資融券約 21:00 後公布；外資、投信連續買賣超天數存在 Redis，同一天重跑不重複累加。
        create_job(
            "0 40 21 * * Mon-Fri",
            "通知持股法人與信用交易",
            event::taiwan_stock::chip_flow::execute,
        ),
        // 21:50（週一至週五）全市場三大法人買賣超與融資融券餘額入庫（chip_daily）
        create_job("0 50 21 * * Mon-Fri", "籌碼資料入庫", chip::execute_daily),
        // 10:40（週六）集保股權分散週摘要入庫（holder_distribution）；開放資料只有最新一週
        create_job(
            "0 40 10 * * Sat",
            "集保股權分散入庫",
            chip::execute_holder_distributions,
        ),
        // 17:30、20:30（週一至週五）以證交所、櫃買中心官方值補最近 7 天日線的本益比與股價淨值比
        // （櫃買當天資料 17:30 常未公布，20:30 再跑一次）
        create_job(
            "0 30 17,20 * * Mon-Fri",
            "補齊官方本益比與股價淨值比",
            official_ratio::execute,
        ),
        // 20:50（每月 10–28 日）董監事持股與設質入庫（insider_holding）；同月份重跑只更新有變的列
        create_job(
            "0 50 20 10-28 * *",
            "董監持股入庫",
            chip::execute_insider_holdings,
        ),
        // 22:10（週一至週五）策略換股通知：依 .env 的 STRATEGY_ALERT_QUERY 跑條件選股回測，
        // 今天是換股日才發；排在 21:50 籌碼入庫之後，外資／投信連續買超條件才看得到今天。
        create_job(
            "0 10 22 * * Mon-Fri",
            "策略換股通知",
            event::taiwan_stock::strategy_alert::execute,
        ),
        // 22:00 外資持股狀態
        create_job(
            "0 0 22 * * *",
            "更新外資持股比例與狀態",
            qualified_foreign_institutional_investor::execute,
        ),
        // 23:00（週一至週五）檢查大盤指數、籌碼與外資持股是否寫入，缺漏就重跑並告警；
        // 另重跑一次官方本益比補寫，有補到才告警（排程失敗時當天的資料不會一直缺著）。
        create_job(
            "0 0 23 * * Mon-Fri",
            "檢查盤後籌碼與外資資料完整性",
            event::taiwan_stock::closing::ensure_after_hours,
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
/// 5. 超過 [`JOB_TIMEOUT`] 仍未結束時中止該輪，記錄 `task.timeout` 並發送告警。
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
            match run_with_timeout(task(), JOB_TIMEOUT).await {
                TaskOutcome::Done => {
                    tracing::info!(
                        task = cron_expr,
                        name = name,
                        elapsed_ms = t.elapsed().as_millis() as u64,
                        "task.done"
                    );
                }
                TaskOutcome::Failed(why) => {
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
                TaskOutcome::TimedOut => {
                    let err_msg = format!(
                        "執行超過 {} 分鐘仍未結束，已中止本輪；請查看日誌中該任務最後的紀錄",
                        JOB_TIMEOUT.as_secs() / 60
                    );
                    tracing::error!(
                        task = cron_expr,
                        name = name,
                        elapsed_ms = t.elapsed().as_millis() as u64,
                        "task.timeout"
                    );
                    alert::send_alert(&format!("排程任務 [{}] 逾時", name), &err_msg).await;
                }
            }
        })
    })?)
}

/// 單輪排程任務的執行時間上限。
///
/// 沒有上限時，任務只要卡住就永遠不會留下 `task.done`／`task.failed`，也不會告警：
/// 2026-09-21 21:00 的股利排程因重試延遲失控（第四次重試要等 24.75 小時）卡了兩天，
/// 直到重新部署才被發現，期間隔天同時段又啟動了新的一輪。
///
/// 3 小時的依據：正式站 2026-09-17～24 各排程最長 63.9 分鐘（21:00 股利採集，
/// 且為修正重試前的數字），Yahoo 財報採集預估約 45 分鐘。上限小於 24 小時，
/// 也保證每日排程不會兩輪重疊。
const JOB_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

/// 單輪排程任務的執行結果。
#[derive(Debug)]
enum TaskOutcome {
    /// 正常結束。
    Done,
    /// 任務回傳錯誤。
    Failed(Error),
    /// 超過時間上限，已中止。
    TimedOut,
}

/// 在 `limit` 內執行任務；逾時即 drop 任務 future 將其中止。
///
/// 中止發生在任務的下一個 `.await` 點：進行中的資料庫交易隨 drop 回滾，
/// 已提交的寫入保留。採集類任務皆為 upsert／逐檔處理，下一輪可以接續。
async fn run_with_timeout<Fut>(task: Fut, limit: Duration) -> TaskOutcome
where
    Fut: Future<Output = Result<(), Error>>,
{
    match tokio::time::timeout(limit, task).await {
        Ok(Ok(())) => TaskOutcome::Done,
        Ok(Err(why)) => TaskOutcome::Failed(why),
        Err(_) => TaskOutcome::TimedOut,
    }
}

#[cfg(test)]
mod tests;
