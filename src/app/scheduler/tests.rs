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
