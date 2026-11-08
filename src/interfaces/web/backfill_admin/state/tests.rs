//! 手動回補工作狀態的測試。

use super::*;

/// 建立一筆指定狀態的測試 job。
fn make_job(id: &str, started_at: &str, status: BackfillJobStatus) -> BackfillJob {
    BackfillJob {
        progress_percent: 0,
        id: id.to_string(),
        kind: "test_kind".to_string(),
        input: "input".to_string(),
        status,
        message: String::new(),
        started_at: started_at.to_string(),
        finished_at: None,
    }
}

/// 驗證修剪只刪最舊的已完成 job，且執行中的 job 一律保留。
#[test]
fn prune_finished_jobs_removes_oldest_finished_only() {
    let mut jobs = HashMap::new();

    // 塞入「保留上限 + 2」筆已完成 job；started_at 遞增，編號越小越舊。
    // 用小數秒帶入序號，確保時間字串的字典序與編號嚴格一致。
    let total_finished = MAX_FINISHED_JOBS_RETAINED + 2;
    for i in 0..total_finished {
        let id = format!("finished-{i:04}");
        let started_at = format!("2026-07-10T00:00:00.{i:04}+08:00");
        jobs.insert(
            id.clone(),
            make_job(&id, &started_at, BackfillJobStatus::Succeeded),
        );
    }
    // 再塞入一筆「比所有已完成 job 都舊」的執行中 job，驗證它不會被刪。
    jobs.insert(
        "running-oldest".to_string(),
        make_job(
            "running-oldest",
            "2026-07-09T00:00:00+08:00",
            BackfillJobStatus::Running,
        ),
    );

    BackfillWebState::prune_finished_jobs(&mut jobs);

    // 已完成 job 應回到上限內；執行中 job 必須存活。
    let finished_count = jobs
        .values()
        .filter(|j| !matches!(j.status, BackfillJobStatus::Running))
        .count();
    assert_eq!(finished_count, MAX_FINISHED_JOBS_RETAINED);
    assert!(jobs.contains_key("running-oldest"));
    // 最舊的兩筆已完成 job 應被刪除。
    assert!(!jobs.contains_key("finished-0000"));
    assert!(!jobs.contains_key("finished-0001"));
}

/// 驗證查重只比對「相同 kind + input 且仍在執行」的 job。
#[test]
fn find_active_job_matches_running_same_kind_and_input() {
    let mut jobs = HashMap::new();
    jobs.insert(
        "done".to_string(),
        make_job(
            "done",
            "2026-07-10T00:00:00+08:00",
            BackfillJobStatus::Succeeded,
        ),
    );
    jobs.insert(
        "running".to_string(),
        make_job(
            "running",
            "2026-07-10T00:00:01+08:00",
            BackfillJobStatus::Running,
        ),
    );

    // 相同 kind+input 且執行中 → 命中。
    assert_eq!(
        BackfillWebState::find_active_job(&jobs, "test_kind", "input"),
        Some("running".to_string())
    );
    // 已完成的 job 不算重複；不同 input 也不算重複。
    assert_eq!(
        BackfillWebState::find_active_job(&jobs, "test_kind", "other"),
        None
    );
    assert_eq!(
        BackfillWebState::find_active_job(&jobs, "other_kind", "input"),
        None
    );
}

/// 驗證併行名額領完後 `try_acquire_job_slot` 回傳 None，歸還後可再領。
#[test]
fn job_slots_enforce_concurrency_limit() {
    let state = BackfillWebState::new();

    // 把名額全部領走。
    let mut permits = Vec::new();
    for _ in 0..MAX_CONCURRENT_JOBS {
        permits.push(
            state
                .try_acquire_job_slot()
                .expect("permit should be available"),
        );
    }
    // 名額用罄，再領應該失敗。
    assert!(state.try_acquire_job_slot().is_none());

    // drop 一張 permit（模擬一個 job 結束），名額立即歸還。
    permits.pop();
    assert!(state.try_acquire_job_slot().is_some());
}
