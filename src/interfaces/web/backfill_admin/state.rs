use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Local;
use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

/// 同時執行中的 manual backfill job 數量上限。
///
/// 每個 job 都會佔用資料庫連線、外部網站流量與記憶體；不設上限時，
/// 惡意或誤觸的大量請求可以無限 spawn 背景 task，最終耗盡 DB pool
/// 或被外部資料來源封鎖。超過上限的請求會被拒絕（HTTP 429 / gRPC
/// RESOURCE_EXHAUSTED），呼叫端稍後重試即可。
pub(super) const MAX_CONCURRENT_JOBS: usize = 4;

/// 已完成（succeeded / failed）job 在記憶體中的保留數量上限。
///
/// job 狀態表存在記憶體 HashMap 中；若永不清理，長時間運行會讓記憶體
/// 持續增長。超過上限時會從「最舊的已完成 job」開始刪除；
/// 執行中的 job 永遠不會被清掉，因此呼叫端不會查不到進行中的工作。
pub(super) const MAX_FINISHED_JOBS_RETAINED: usize = 100;

/// Backfill admin Web API 共用狀態。
///
/// 狀態目前保存在記憶體中，適合單一程序內的臨時手動維運用途。
#[derive(Clone)]
pub(super) struct BackfillWebState {
    /// 以 job id 為 key 的回補工作表。
    pub(super) jobs: Arc<RwLock<HashMap<String, BackfillJob>>>,
    /// 產生同一秒內多筆 job id 的遞增序號。
    next_id: Arc<AtomicU64>,
    /// 全域併行度閘門：一個 permit 代表一個「可同時執行的 job 名額」。
    ///
    /// Semaphore（號誌）可以想成一疊固定張數的入場券：job 啟動前先領一張，
    /// job 結束（不論成功、失敗、逾時）時歸還。領不到就表示名額已滿。
    job_slots: Arc<Semaphore>,
}

/// Backfill admin 的全域記憶體狀態。
pub(super) static BACKFILL_STATE: Lazy<BackfillWebState> = Lazy::new(BackfillWebState::new);

impl BackfillWebState {
    /// 建立空的 job 狀態容器。
    pub(super) fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            job_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS)),
        }
    }

    /// 產生新的 job id。
    ///
    /// 格式為 `yyyyMMddHHmmss-seq`，同時保留時間排序資訊與單程序內唯一性。
    pub(super) fn next_job_id(&self) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("{}-{seq}", Local::now().format("%Y%m%d%H%M%S"))
    }

    /// 嘗試領取一個 job 併行名額，領不到（已達上限）時回傳 `None`。
    ///
    /// 用 `try_acquire`（立即返回）而不是 `acquire`（排隊等待）：
    /// 手動回補 API 的語意是「滿了就直接告訴呼叫端」，
    /// 而不是讓 HTTP request 掛著等其他 job 結束。
    /// 回傳的 `OwnedSemaphorePermit` 是 RAII 憑證——它被 drop 的那一刻
    /// 名額自動歸還，因此只要把 permit move 進背景 task，
    /// 不論 task 正常結束、錯誤或 panic，名額都不會遺失。
    pub(super) fn try_acquire_job_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.job_slots).try_acquire_owned().ok()
    }

    /// 在（呼叫端已鎖定的）job 表中尋找相同 `(kind, input)` 且仍在執行的 job。
    ///
    /// 用途：同一個交易日的回補若同時跑兩份，會互相刪除/覆寫資料，
    /// 因此建立前先查重。呼叫端必須自己持有 `jobs` 的鎖再把 map 傳進來，
    /// 讓「查重」與「登記」發生在同一次持鎖期間，避免兩個並發請求
    /// 同時通過查重（check-then-act 競態）。
    pub(super) fn find_active_job(
        jobs: &HashMap<String, BackfillJob>,
        kind: &str,
        input: &str,
    ) -> Option<String> {
        jobs.values()
            .find(|job| {
                matches!(job.status, BackfillJobStatus::Running)
                    && job.kind == kind
                    && job.input == input
            })
            .map(|job| job.id.clone())
    }

    /// 修剪已完成的 job，讓保留數量不超過 [`MAX_FINISHED_JOBS_RETAINED`]。
    ///
    /// 只刪「已完成」的 job（succeeded/failed），並且從最舊的開始刪
    /// （job 的 `started_at` 為 RFC 3339 字串，字典序即時間序）。
    /// 執行中的 job 一律保留，呼叫端永遠查得到進行中的工作。
    pub(super) fn prune_finished_jobs(jobs: &mut HashMap<String, BackfillJob>) {
        // 收集所有已完成 job 的 (started_at, id)，準備依時間排序。
        let mut finished: Vec<(String, String)> = jobs
            .values()
            .filter(|job| !matches!(job.status, BackfillJobStatus::Running))
            .map(|job| (job.started_at.clone(), job.id.clone()))
            .collect();

        // 未超過保留上限就不需要動作。
        if finished.len() <= MAX_FINISHED_JOBS_RETAINED {
            return;
        }

        // 依 started_at 由舊到新排序，刪掉最舊的那些，直到回到上限內。
        finished.sort();
        let remove_count = finished.len() - MAX_FINISHED_JOBS_RETAINED;
        for (_, id) in finished.into_iter().take(remove_count) {
            jobs.remove(&id);
        }
    }
}

/// Manual backfill job 的執行狀態。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackfillJobStatus {
    /// 已建立且背景 task 正在執行。
    Running,
    /// 背景 task 已完成且回傳成功。
    Succeeded,
    /// 背景 task 執行失敗，錯誤訊息會寫入 `BackfillJob::message`。
    Failed,
}

/// Manual backfill job 的查詢模型。
///
/// 此結構同時供 Web API 回應與 gRPC 轉換使用，因此可見度限制在 crate 內。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BackfillJob {
    /// Job 唯一識別碼。
    pub(crate) id: String,
    /// Job 類型，例如 `closing_aggregate`。
    pub(crate) kind: String,
    /// Job 輸入參數的可讀字串。
    pub(crate) input: String,
    /// Job 目前狀態。
    pub(super) status: BackfillJobStatus,
    /// Job 狀態說明或完成/失敗訊息。
    pub(crate) message: String,
    /// Job 建立時間，使用 RFC 3339 字串。
    pub(crate) started_at: String,
    /// Job 完成時間，尚未完成時為 `None`。
    pub(crate) finished_at: Option<String>,
}

impl BackfillJob {
    /// 回傳對外 API 使用的 snake_case 狀態標籤。
    pub(crate) fn status_label(&self) -> &'static str {
        match self.status {
            BackfillJobStatus::Running => "running",
            BackfillJobStatus::Succeeded => "succeeded",
            BackfillJobStatus::Failed => "failed",
        }
    }
}

/// 取得所有 manual backfill jobs，並依建立時間由新到舊排序。
pub(crate) async fn list_backfill_jobs() -> Vec<BackfillJob> {
    let mut jobs = BACKFILL_STATE
        .jobs
        .read()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    jobs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    jobs
}

/// 依 job id 取得 manual backfill job。
pub(crate) async fn get_backfill_job(id: &str) -> Option<BackfillJob> {
    BACKFILL_STATE.jobs.read().await.get(id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建立一筆指定狀態的測試 job。
    fn make_job(id: &str, started_at: &str, status: BackfillJobStatus) -> BackfillJob {
        BackfillJob {
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
}
