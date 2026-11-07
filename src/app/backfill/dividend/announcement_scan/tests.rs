//! 除權除息公告掃描對真實來源的乾跑（#[ignore]，手動執行）。

use super::*;

/// 對真實來源做一次不碰資料庫的乾跑，印出配對統計。
///
/// `existing` 刻意給空的，因此每一筆公告都會走「新增或無法判定」這條路徑，
/// 可以直接看出第一階段（MOPS）能配對出多少、剩下多少要交給第二階段。
/// 這支測試只讀取外部資料，不寫入任何東西，也不會呼叫 Yahoo。
#[tokio::test]
#[ignore]
async fn test_dry_run_without_database() {
    dotenvy::dotenv().ok();

    let (listed, otc, allotments, schedules) = tokio::join!(
        twse::ex_dividend_announcement::visit(),
        tpex::ex_dividend_announcement::visit(),
        dividend_allotment::visit(),
        dividend_schedule::visit(),
    );

    let mut announcements = listed.expect("上市預告表應可取得");
    announcements.extend(otc.expect("上櫃預告表應可取得"));
    let allotments = index_allotments(allotments.expect("股利分派情形應可取得"));
    let schedules = index_schedules(schedules.expect("股利政策表應可取得"));

    let plan = build_scan_plan(&announcements, &allotments, &schedules, &HashMap::new());

    println!(
        "公告 {} 筆／第一階段可新增 {} 筆／待 Yahoo 補救 {} 筆",
        announcements.len(),
        plan.inserts.len(),
        plan.unresolved.len()
    );
    let with_payable_date = plan
        .inserts
        .values()
        .filter(|item| item.payable_date_cash != UNANNOUNCED_DATE)
        .count();
    println!("可新增的資料中已帶現金發放日 {} 筆", with_payable_date);
}
