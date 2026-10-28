//! 週／月資料：千張大戶、集保級距、400 張欄位與董監持股的檢查。

use super::*;

/// 集保級距落後千張大戶、列數太少、千張大戶最新週有 400 張空值都要列出；沒有級距資料時不附數字。
#[test]
fn holder_levels_must_follow_the_weekly_summary() {
    let mut snapshot = healthy();
    snapshot.latest_holder_week = Some(day(9));
    snapshot.latest_holder_level_week = Some(day(2));
    snapshot.holder_level_rows = 41_000;
    snapshot.holders_400_missing = 3;
    assert_eq!(
        periodic_table_problems(&snapshot, day(10)),
        [
            "holder_distribution_level 最新 2026-10-02，落後 holder_distribution（2026-10-09）",
            "holder_distribution_level 2026-10-02 只有 41000 列，應至少 60000 列",
            "holder_distribution 2026-10-09 有 3 檔 holders_400 是空值",
        ]
    );
    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
    let message = report(day(4), day(10), &checks);
    assert!(
        message.contains(
            "⚠️ 週／月資料（集保級距 2026\\-10\\-02 41000 列、400 張空值 3 檔）\n  • holder"
        ),
        "{message}"
    );

    snapshot.latest_holder_level_week = None;
    snapshot.holder_level_rows = 0;
    snapshot.holders_400_missing = 0;
    assert_eq!(
        periodic_table_problems(&snapshot, day(10)),
        ["holder_distribution_level 沒有任何資料"]
    );
    assert_eq!(holder_level_detail(&snapshot), None);

    // 千張大戶本身沒有資料時只報那一項，不再追究級距。
    snapshot.latest_holder_week = None;
    assert_eq!(
        periodic_table_problems(&snapshot, day(10))[0],
        "holder_distribution 沒有任何資料"
    );
    assert_eq!(periodic_table_problems(&snapshot, day(10)).len(), 1);
}

/// 千張大戶超過 9 天、董監持股早於 55 天前那個月才算停更；沒有資料也列出。
#[test]
fn periodic_tables_must_not_go_stale() {
    let mut snapshot = healthy();
    // 10-10 往前 9 天是 10-01（剛好到期限不算停更）；55 天前是 08-16，應至少有 8 月。
    snapshot.latest_holder_week = Some(day(1));
    assert!(periodic_table_problems(&snapshot, day(10)).is_empty());
    snapshot.latest_holder_week = NaiveDate::from_ymd_opt(2026, 9, 30);
    snapshot.latest_insider_month = NaiveDate::from_ymd_opt(2026, 7, 1);
    assert_eq!(
        periodic_table_problems(&snapshot, day(10)),
        [
            "holder_distribution 最新 2026-09-30，已超過 9 天沒更新",
            "insider_holding 最新 2026-07，應已有 2026-08",
        ]
    );
    snapshot.latest_holder_week = None;
    snapshot.latest_insider_month = None;
    assert_eq!(
        problems_of(
            &evaluate(&snapshot, day(4), day(10), Some(&[])),
            "週／月資料"
        ),
        [
            "holder_distribution 沒有任何資料",
            "insider_holding 沒有任何資料"
        ]
    );
}
