use std::sync::atomic::{AtomicUsize, Ordering};

use jiff::civil::{Weekday, date};

use super::*;

/// 以臺灣時間建立時間點。
fn taipei_at(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> Timestamp {
    taipei()
        .to_timestamp(date(year, month, day).at(hour, minute, second, 0))
        .expect("合法的臺灣時間")
}

fn noop_job(expression: &'static str) -> Result<CronJob> {
    CronJob::new(expression, "test", || async {})
}

#[test]
fn next_after_reads_expression_in_taipei_time() {
    // 2026-11-22 是星期日，15:29（臺灣時間）
    let sunday = taipei_at(2026, 11, 22, 15, 29, 0);
    let next = noop_job("0 30 15 * * Mon-Fri")
        .expect("合法")
        .next_after(sunday)
        .expect("有下一次");
    assert_eq!(next.date(), date(2026, 11, 23), "週日之後的第一個平日");
    assert_eq!((next.hour(), next.minute(), next.second()), (15, 30, 0));
    assert_eq!(next.date().weekday(), Weekday::Monday);
    assert_eq!(next.offset(), jiff::tz::offset(8));

    // 「之後」不含當下：正好在觸發時間時，下一次是隔天
    let at_trigger = taipei_at(2026, 11, 23, 15, 30, 0);
    let after = noop_job("0 30 15 * * Mon-Fri")
        .expect("合法")
        .next_after(at_trigger)
        .expect("有下一次");
    assert_eq!(after.date(), date(2026, 11, 24));
}

#[test]
fn day_range_and_step_expressions() {
    let start = taipei_at(2026, 11, 28, 21, 0, 0);
    let pledge = noop_job("0 45 20 10-28 * *")
        .expect("合法")
        .next_after(start)
        .expect("有下一次");
    assert_eq!(
        pledge.date(),
        date(2026, 12, 10),
        "28 日之後跳到下個月 10 日"
    );
    let every_five = noop_job("30 */5 * * * *")
        .expect("合法")
        .next_after(start)
        .expect("有下一次");
    assert_eq!((every_five.minute(), every_five.second()), (0, 30));
    let every_six_hours = noop_job("0 5 */6 * * *")
        .expect("合法")
        .next_after(start)
        .expect("有下一次");
    assert_eq!(
        (every_six_hours.date(), every_six_hours.hour()),
        (date(2026, 11, 29), 0)
    );
    let first_days = noop_job("0 30 7 1-10 * *")
        .expect("合法")
        .next_after(start)
        .expect("有下一次");
    assert_eq!(first_days.date(), date(2026, 12, 1));
    let saturday = noop_job("0 20 10 * * Sat")
        .expect("合法")
        .next_after(start)
        .expect("有下一次");
    assert_eq!(saturday.date(), date(2026, 12, 5));
}

#[test]
fn invalid_expressions_are_rejected() {
    for expression in ["0 40 20 * * Xyz", "0 40 20 * *", "61 * * * * *", ""] {
        let error = noop_job(expression).expect_err(expression);
        assert!(
            error.to_string().contains("test"),
            "錯誤訊息帶任務名稱：{error}"
        );
    }
    let job = noop_job("0 30 15 * * Mon-Fri").expect("合法");
    assert_eq!(
        (job.name(), job.expression()),
        ("test", "0 30 15 * * Mon-Fri")
    );
    assert!(format!("{job:?}").contains("0 30 15 * * Mon-Fri"));
}

/// 每秒觸發的任務會執行；shutdown 之後不再觸發。
#[tokio::test]
async fn scheduler_fires_until_shutdown() {
    let count = Arc::new(AtomicUsize::new(0));
    let scheduler = CronScheduler::new();
    let counter = Arc::clone(&count);
    scheduler.add(
        CronJob::new("* * * * * *", "every second", move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        })
        .expect("合法"),
    );
    scheduler.start();
    tokio::time::sleep(Duration::from_millis(2_300)).await;
    scheduler.shutdown();
    let fired = count.load(Ordering::SeqCst);
    assert!(fired >= 1, "2.3 秒內應至少觸發一次，實際 {fired}");
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(count.load(Ordering::SeqCst), fired, "shutdown 後不再觸發");
}

/// 觸發後的任務登記為進行中，關機時會被等待。
#[tokio::test]
async fn fired_job_is_tracked_as_background_operation() {
    let release = Arc::new(tokio::sync::Notify::new());
    let started = Arc::new(tokio::sync::Notify::new());
    let scheduler = CronScheduler::new();
    let (job_release, job_started) = (Arc::clone(&release), Arc::clone(&started));
    scheduler.add(
        CronJob::new("* * * * * *", "tracked", move || {
            let (release, started) = (Arc::clone(&job_release), Arc::clone(&job_started));
            async move {
                started.notify_one();
                release.notified().await;
            }
        })
        .expect("合法"),
    );
    scheduler.start();
    tokio::time::timeout(Duration::from_secs(3), started.notified())
        .await
        .expect("3 秒內應觸發");
    scheduler.shutdown();
    assert!(
        BACKGROUND_OPERATIONS.active_count() >= 1,
        "執行中的任務要登記"
    );
    release.notify_waiters();
}
