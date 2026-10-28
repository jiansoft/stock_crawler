//! ETF 淨值漏抓、正式淨值回填與基本資料停更的檢查。

use super::*;

/// ETF 淨值漏抓、正式淨值沒回填與基本資料停更都要列出；休市日的海外 ETF 淨值、
/// 最新交易日還沒回填的正式淨值不算。
#[test]
fn etf_nav_gaps_and_stale_profiles_are_flagged() {
    let mut snapshot = healthy();
    snapshot.etf_nav_counts.retain(|count| count.date != day(6));
    for count in &mut snapshot.etf_nav_counts {
        match count.date.day() {
            7 => count.rows = 320,
            8 => count.missing_nav = 40,
            _ => {}
        }
    }
    // 10-10（六）只有海外 ETF：不是交易日，不列。
    snapshot.etf_nav_counts.push(etf_nav(day(10), 45, 45));
    snapshot.stale_etf_profiles = (1..=12).map(|n| format!("009{n:02}")).collect();

    let checks = evaluate(&snapshot, day(4), day(11), Some(&[]));
    let problems = problems_of(&checks, "ETF 資料");
    assert_eq!(
        problems,
        [
            "2026-10-06 沒有 ETF 淨值".to_owned(),
            "2026-10-07 ETF 淨值只有 320 檔（近期 360 檔）".to_owned(),
            "2026-10-08 有 40／360 檔 ETF 沒有正式淨值（下一個營業日的回填沒成功）".to_owned(),
            "12 檔 ETF 基本資料超過 9 天沒更新：00901、00902、00903、00904、00905、00906、00907、00908、00909、00910…".to_owned(),
        ]
    );
}

/// 沒有基準期的 ETF 淨值（剛開始抓）時只檢查整天缺漏。
#[test]
fn etf_nav_without_baseline_only_checks_missing_days() {
    let mut snapshot = healthy();
    snapshot.etf_nav_counts.retain(|count| count.date != day(2));
    snapshot.etf_nav_counts[0].rows = 10;
    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
    assert!(problems_of(&checks, "ETF 資料").is_empty());
}
