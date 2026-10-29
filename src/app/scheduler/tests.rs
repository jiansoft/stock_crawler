// 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
use super::*;

/// 建立測試用排程器並註冊每秒任務。
async fn run() -> Result<()> {
    let sched = JobScheduler::new().await?;
    let every_minute = Job::new_async("* * * * * *", |_uuid, _l| {
        Box::pin(async move {
            tracing::debug!("每秒任務 now: {:?}", chrono::Local::now());
        })
    })?;
    sched.add(every_minute).await?;

    sched.start().await?;

    Ok(())
}

#[tokio::test]
async fn run_with_timeout_reports_done() {
    let outcome = run_with_timeout(async { Ok(()) }, Duration::from_secs(1)).await;
    assert!(matches!(outcome, TaskOutcome::Done));
}

#[tokio::test]
async fn run_with_timeout_reports_failure() {
    let outcome = run_with_timeout(
        async { Err(anyhow::anyhow!("boom")) },
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(outcome, TaskOutcome::Failed(why) if why.to_string() == "boom"));
}

/// 排程字串解析失敗的任務會在註冊時被 `flatten` 略過而不報錯，
/// 因此用到星期、日期範圍的排程要確認能解析，且下次觸發落在預期的台北時間。
#[tokio::test]
async fn range_cron_expressions_parse_and_fire_in_taipei_time() {
    use chrono::{Datelike, Timelike};

    let taipei = FixedOffset::east_opt(8 * 3600).unwrap();
    let mut sched = JobScheduler::new().await.expect("建立排程器");
    for (expr, hour, minute) in [
        ("0 30 15 * * Mon-Fri", 15, 30),
        ("0 40 20 * * Mon-Fri", 20, 40),
        ("0 45 20 10-28 * *", 20, 45),
    ] {
        let job = create_job(expr, "test", || async { Ok(()) }).expect(expr);
        let job_id = sched.add(job).await.expect("加入排程");
        let next = sched
            .next_tick_for_job(job_id)
            .await
            .expect("取得下次觸發時間")
            .expect("應有下次觸發時間")
            .with_timezone(&taipei);
        assert_eq!((next.hour(), next.minute()), (hour, minute), "{expr}");
        if expr.ends_with("Mon-Fri") {
            assert!(next.weekday().number_from_monday() <= 5, "{expr}: {next}");
        } else {
            assert!((10..=28).contains(&next.day()), "{expr}: {next}");
        }
    }
    assert!(create_job("0 40 20 * * Xyz", "test", || async { Ok(()) }).is_err());
}

/// 每 10 分鐘的主機健康檢查落在整十分，23:30 的彙總落在台北時間 23:30。
#[tokio::test]
async fn host_health_cron_expressions_fire_on_expected_minutes() {
    use chrono::Timelike;

    let taipei = FixedOffset::east_opt(8 * 3600).unwrap();
    let mut sched = JobScheduler::new().await.expect("建立排程器");
    let quiet = create_quiet_job("0 */10 * * * *", "test", || async { Ok(()) }).expect("*/10");
    let summary = create_job("0 30 23 * * *", "test", || async { Ok(()) }).expect("23:30");
    let quiet_id = sched.add(quiet).await.expect("加入排程");
    let summary_id = sched.add(summary).await.expect("加入排程");
    let quiet_next = sched
        .next_tick_for_job(quiet_id)
        .await
        .expect("取得下次觸發時間")
        .expect("應有下次觸發時間")
        .with_timezone(&taipei);
    assert_eq!(
        (quiet_next.minute() % 10, quiet_next.second()),
        (0, 0),
        "{quiet_next}"
    );
    let summary_next = sched
        .next_tick_for_job(summary_id)
        .await
        .expect("取得下次觸發時間")
        .expect("應有下次觸發時間")
        .with_timezone(&taipei);
    assert_eq!((summary_next.hour(), summary_next.minute()), (23, 30));
}

/// 每 5 分鐘的資料庫健康檢查落在 5 的倍數分的第 30 秒，避開整分啟動的排程。
#[tokio::test]
async fn db_health_cron_expression_fires_at_half_minute() {
    use chrono::Timelike;

    let taipei = FixedOffset::east_opt(8 * 3600).unwrap();
    let mut sched = JobScheduler::new().await.expect("建立排程器");
    let job = create_quiet_job("30 */5 * * * *", "test", || async { Ok(()) }).expect("*/5");
    let job_id = sched.add(job).await.expect("加入排程");
    let next = sched
        .next_tick_for_job(job_id)
        .await
        .expect("取得下次觸發時間")
        .expect("應有下次觸發時間")
        .with_timezone(&taipei);

    assert_eq!((next.minute() % 5, next.second()), (0, 30), "{next}");
}

/// 一輪排程的成功、失敗與逾時都能走完（失敗與逾時的告警在未註冊 sink 時降為 warning log）。
#[tokio::test]
async fn run_job_completes_for_every_outcome() {
    for quiet in [false, true] {
        run_job(
            "0 0 0 * * *",
            "test",
            quiet,
            Duration::from_secs(1),
            || async { Ok(()) },
        )
        .await;
    }
    run_job(
        "0 0 0 * * *",
        "test",
        true,
        Duration::from_secs(1),
        || async { Err(anyhow::anyhow!("boom")) },
    )
    .await;
    let started = Instant::now();
    run_job(
        "0 0 0 * * *",
        "test",
        false,
        Duration::from_millis(20),
        || async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            Ok(())
        },
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// 策略優勢滾動檢查掛在每月 1–10 日的台北時間 07:30（是否為第一個交易日由任務自己判斷）。
#[tokio::test]
async fn strategy_edge_cron_fires_in_first_ten_days() {
    use chrono::{Datelike, Timelike};

    let taipei = FixedOffset::east_opt(8 * 3600).unwrap();
    let mut sched = JobScheduler::new().await.expect("建立排程器");
    let job = create_job("0 30 7 1-10 * *", "test", || async { Ok(()) }).expect("1-10");
    let job_id = sched.add(job).await.expect("加入排程");
    let next = sched
        .next_tick_for_job(job_id)
        .await
        .expect("取得下次觸發時間")
        .expect("應有下次觸發時間")
        .with_timezone(&taipei);
    assert_eq!((next.hour(), next.minute()), (7, 30), "{next}");
    assert!((1..=10).contains(&next.day()), "{next}");
}

/// 卡住的任務必須在上限到時被中止，而不是無限期等下去。
#[tokio::test]
async fn run_with_timeout_aborts_hung_task() {
    let started = Instant::now();
    let outcome = run_with_timeout(
        async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            Ok(())
        },
        Duration::from_millis(50),
    )
    .await;

    assert!(matches!(outcome, TaskOutcome::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(5));
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

/// 固定拆檔前的全部任務名稱與 cron，並實際建構每個任務驗證沒有遺漏或解析失敗。
#[test]
fn registered_jobs_preserve_names_and_cron() {
    let expected = [
        ("0 0 1 * * *", "更新興櫃股票每股淨值"),
        ("0 30 2 * * *", "更新盈餘分配率"),
        ("0 0 3 * * *", "更新台股季度財報"),
        ("0 0 4 * * *", "補齊季報零淨值之 ROE/ROA 數據"),
        ("0 0 5 * * *", "更新年度財報基本 EPS"),
        ("0 5 5 * * *", "更新完整年度財報"),
        ("0 10 5 * * *", "更新每股淨值(補 Yahoo 零淨值數據)"),
        ("0 15 5 * * *", "取得台股月度營收"),
        ("0 20 5 * * *", "更新台股 ISIN 識別碼"),
        ("0 25 5 * * *", "更新下市股票清單"),
        ("0 30 5 * * *", "更新台股 ETF 資訊"),
        ("0 32 5 * * *", "回補交易所公告的減資事件"),
        ("0 35 5 * * *", "掃描除權息公告補齊漏抓事件"),
        ("0 37 5 * * *", "核對交易所除權息結果"),
        ("0 40 5 * * *", "計算各期間年化報酬率(CAGR)"),
        ("0 0 6 * * *", "採集 Yahoo 三大財務報表"),
        ("0 10 7 * * *", "重算財務體質評分"),
        ("0 30 7 1-10 * *", "策略優勢滾動檢查"),
        ("0 0 8 * * *", "提醒本日與明日除權息股票"),
        ("0 2 8 * * *", "提醒本日持股股利發放"),
        ("0 4 8 * * *", "提醒本日公開申購股票"),
        ("0 0 9 * * *", "更新股票加權權值佔比"),
        ("0 2 9 * * *", "監控並提醒股價達高低標"),
        ("0 0 15 * * *", "取得每日收盤行情與報價"),
        ("0 30 15 * * Mon-Fri", "通知大盤趨勢變化"),
        ("0 0 17,19 * * Mon-Fri", "檢查收盤資料完整性"),
        ("0 20 17 * * Mon-Fri", "通知持股填息"),
        ("0 30 17 * * Mon-Fri", "更新 ETF 淨值與專區快照"),
        ("0 45 17 * * Mon-Fri", "通知持有 ETF 折溢價與申贖"),
        ("0 30 20 * * *", "通知持股法說會摘要"),
        ("0 40 20 * * Mon-Fri", "通知持股主力進出"),
        ("0 45 20 10-28 * *", "通知持股董監質押變動"),
        ("0 20 10 * * Sat", "每週市場摘要"),
        ("0 50 10 * * Sat", "通知持股估值週報"),
        ("0 10 12 * * Sat", "更新 ETF 基本資料與持股"),
        ("0 5 13 * * Sat", "補 ETF 淨值缺日"),
        ("0 0 11 * * Sat", "資料健康週報"),
        ("0 15 19 * * Mon-Fri", "通知持股技術訊號"),
        ("0 0 21 * * *", "補齊缺失之年度配息數據"),
        ("0 40 21 * * Mon-Fri", "通知持股法人與信用交易"),
        ("0 50 21 * * Mon-Fri", "籌碼資料入庫"),
        ("0 40 10 * * Sat", "集保股權分散入庫與持股大戶週報"),
        ("0 30 17,20 * * Mon-Fri", "補齊官方本益比與股價淨值比"),
        ("0 50 20 10-28 * *", "董監持股入庫"),
        ("0 5 22 * * Mon-Fri", "重建 ETF 專區快照"),
        ("0 10 22 * * Mon-Fri", "策略換股通知"),
        ("0 0 22 * * *", "更新外資持股比例與狀態"),
        ("0 0 23 * * Mon-Fri", "檢查盤後籌碼與外資資料完整性"),
        ("0 */10 * * * *", "主機健康檢查"),
        ("30 */5 * * * *", "資料庫健康檢查"),
        ("0 30 23 * * *", "主機健康彙總"),
    ];
    let definitions = jobs::definitions();
    let actual: Vec<_> = definitions.iter().map(|job| (job.cron, job.name)).collect();
    assert_eq!(actual, expected);
    for definition in definitions {
        assert!((definition.create)().is_ok(), "{}", definition.name);
    }
}
