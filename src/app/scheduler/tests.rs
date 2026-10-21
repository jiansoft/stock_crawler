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
