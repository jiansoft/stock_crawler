use chrono::{NaiveDate, TimeZone};

use super::*;

fn at(hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 29)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .expect("固定日期")
}

fn local(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 10, 29, hour, minute, second)
        .single()
        .expect("台北時間沒有夏令時間，時間唯一")
}

fn up(started: DateTime<Local>, reset: Option<DateTime<Local>>) -> Result<ServerStatus, String> {
    Ok(ServerStatus {
        started_at: started,
        stats_reset: reset,
    })
}

fn down() -> Result<ServerStatus, String> {
    Err("連線失敗: Connection refused".to_string())
}

/// 服務剛啟動的第一次探測只記下啟動時間，不通知。
#[test]
fn first_success_only_records_start_time() {
    let mut watch = Watch::default();

    let notice = watch.observe(up(local(6, 52, 57), None), at(22, 0));

    assert_eq!(notice, None);
    assert_eq!(watch.started_at, Some(local(6, 52, 57)));
}

/// 只失敗一次（網路抖動）不告警，恢復時也不通知。
#[test]
fn single_failure_is_silent() {
    let mut watch = Watch::default();
    watch.observe(up(local(6, 52, 57), None), at(22, 40));

    assert_eq!(watch.observe(down(), at(22, 45)), None);
    assert_eq!(watch.observe(up(local(6, 52, 57), None), at(22, 50)), None);
    assert_eq!(watch.failures, 0);
    assert_eq!(watch.down_since, None);
}

/// 連續兩次失敗告警，訊息帶第一次失敗時間、次數與錯誤。
#[test]
fn second_consecutive_failure_alerts() {
    let mut watch = Watch::default();

    assert_eq!(watch.observe(down(), at(22, 50)), None);
    let notice = watch.observe(down(), at(22, 55)).expect("第二次失敗要告警");

    assert_eq!(notice.title, "資料庫連不上");
    assert!(notice.message.contains("10-29 22:50"), "{}", notice.message);
    assert!(notice.message.contains("連續 2 次"), "{}", notice.message);
    assert!(
        notice.message.contains("Connection refused"),
        "{}",
        notice.message
    );
}

/// 持續連不上時，告警間隔至少 60 分鐘。
#[test]
fn down_alert_respects_cooldown() {
    let mut watch = Watch::default();
    watch.observe(down(), at(20, 0));
    assert!(watch.observe(down(), at(20, 5)).is_some());

    assert_eq!(watch.observe(down(), at(20, 10)), None);
    assert_eq!(watch.observe(down(), at(21, 0)), None);
    let again = watch
        .observe(down(), at(21, 5))
        .expect("滿 60 分鐘要再告警");

    assert!(again.message.contains("連續 5 次"), "{}", again.message);
    assert!(again.message.contains("10-29 20:00"), "{}", again.message);
}

/// 告警過才在恢復時通知，附連不上的時間長度；啟動時間沒變就不算重啟。
#[test]
fn recovery_after_alert_notifies_duration() {
    let mut watch = Watch::default();
    watch.observe(up(local(6, 52, 57), None), at(22, 40));
    watch.observe(down(), at(22, 45));
    watch.observe(down(), at(22, 50));

    let notice = watch
        .observe(up(local(6, 52, 57), None), at(23, 5))
        .expect("告警過要通知恢復");

    assert_eq!(notice.title, "資料庫已恢復");
    assert!(notice.message.contains("約 20 分鐘"), "{}", notice.message);
    assert_eq!(watch.last_down_alert, None);
}

/// 停機後重啟、統計在啟動時重置：恢復與重啟併成一則，判斷為非正常關機。
#[test]
fn restart_with_stats_reset_reports_unclean_shutdown() {
    let mut watch = Watch::default();
    watch.observe(up(local(6, 52, 57), Some(local(6, 52, 57))), at(22, 40));
    watch.observe(down(), at(22, 50));
    watch.observe(down(), at(22, 55));

    let notice = watch
        .observe(up(local(22, 55, 32), Some(local(22, 55, 32))), at(23, 0))
        .expect("重啟要通知");

    assert_eq!(notice.title, "資料庫重啟過");
    assert!(notice.message.contains("已恢復連線"), "{}", notice.message);
    assert!(
        notice.message.contains("10-29 22:55:32"),
        "{}",
        notice.message
    );
    assert!(
        notice.message.contains("上次啟動 10-29 06:52"),
        "{}",
        notice.message
    );
    assert!(notice.message.contains("非正常關機"), "{}", notice.message);
}

/// 兩次探測之間就重啟完（沒有失敗紀錄）：只發重啟；統計保留代表正常關機。
#[test]
fn quick_restart_with_preserved_stats_reports_clean_shutdown() {
    let mut watch = Watch::default();
    watch.observe(up(local(6, 52, 57), Some(local(1, 0, 0))), at(22, 40));

    let notice = watch
        .observe(up(local(22, 43, 0), Some(local(1, 0, 0))), at(22, 45))
        .expect("重啟要通知");

    assert_eq!(notice.title, "資料庫重啟過");
    assert!(!notice.message.contains("已恢復連線"), "{}", notice.message);
    assert!(notice.message.contains("正常關機"), "{}", notice.message);
    assert!(!notice.message.contains("非正常"), "{}", notice.message);
}

/// 讀不到 pg_stat_statements 時說明無法判斷。
#[test]
fn restart_without_stats_says_unknown() {
    let mut watch = Watch::default();
    watch.observe(up(local(6, 52, 57), None), at(22, 40));

    let notice = watch
        .observe(up(local(22, 43, 0), None), at(22, 45))
        .expect("重啟要通知");

    assert!(notice.message.contains("無法判斷"), "{}", notice.message);
}

/// 當日彙總：失敗次數、最長連不上分鐘數、重啟次數與啟動時間；換日歸零。
#[test]
fn summary_counts_today_and_resets_next_day() {
    let mut watch = Watch::default();
    assert_eq!(watch.summary(at(0, 0).date()), None);

    watch.observe(up(local(6, 52, 57), None), at(22, 40));
    watch.observe(down(), at(22, 45));
    watch.observe(down(), at(22, 50));
    watch.observe(up(local(22, 55, 32), None), at(22, 58));

    let today = at(0, 0).date();
    assert_eq!(
        watch.summary(today).as_deref(),
        Some("資料庫：連不上 2 次（最長約 13 分鐘）、重啟 1 次、啟動於 10-29 22:55")
    );

    let tomorrow = today.succ_opt().expect("隔天");
    let next_morning = tomorrow.and_hms_opt(0, 5, 0).expect("時間");
    watch.observe(up(local(22, 55, 32), None), next_morning);
    assert_eq!(
        watch.summary(tomorrow).as_deref(),
        Some("資料庫：連不上 0 次（最長約 0 分鐘）、重啟 0 次、啟動於 10-29 22:55")
    );
    assert_eq!(watch.summary(today), None);
}

/// 還沒連上過時，彙總寫啟動時間未知。
#[test]
fn summary_without_start_time() {
    let mut watch = Watch::default();
    watch.observe(down(), at(9, 0));

    assert_eq!(
        watch.summary(at(0, 0).date()).as_deref(),
        Some("資料庫：連不上 1 次（最長約 0 分鐘）、重啟 0 次、啟動時間未知")
    );
}

/// 排程入口連得上或連不上都不會回錯誤，彙總讀得到當天紀錄。
#[tokio::test]
async fn execute_never_fails() {
    dotenvy::dotenv().ok();

    execute().await.expect("探測失敗只告警，不回錯誤");

    assert!(daily_summary(Local::now().date_naive()).is_some());
}
