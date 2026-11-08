//! 「每個鍵每天只記一次」的紀錄表的測試。

use super::*;

/// 同一個鍵同一天只算第一次，換日後重新計算。
#[test]
fn first_today_resets_on_day_change() {
    let day1 = NaiveDate::from_ymd_opt(2001, 1, 2).expect("valid date");
    let day2 = NaiveDate::from_ymd_opt(2001, 1, 3).expect("valid date");

    let mut seen = DailySeen::default();

    assert!(seen.first_today("79965", day1));
    assert!(!seen.first_today("79965", day1));
    assert!(seen.first_today("79966", day1));
    assert!(seen.first_today("79965", day2));
    assert!(!seen.first_today("79965", day2));
}

#[test]
fn concurrent_first_today_reports_once_per_key_and_day() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let seen = ConcurrentDailySeen::default();
    let firsts = AtomicUsize::new(0);
    let day = NaiveDate::from_ymd_opt(2026, 10, 23).expect("日期合法");
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                for _ in 0..100 {
                    if seen.first_today("2330|Yahoo", day) {
                        firsts.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    assert_eq!(firsts.load(Ordering::Relaxed), 1);
    assert!(seen.first_today("2330|HiStock", day));
    assert!(seen.first_today("2330|Yahoo", day.succ_opt().expect("隔天合法")));
    assert!(!seen.first_today("2330|Yahoo", day.succ_opt().expect("隔天合法")));
}
