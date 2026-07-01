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
use tokio::sync::RwLock;

/// Backfill admin Web API 共用狀態。
///
/// 狀態目前保存在記憶體中，適合單一程序內的臨時手動維運用途。
#[derive(Clone)]
pub(super) struct BackfillWebState {
    /// 以 job id 為 key 的回補工作表。
    pub(super) jobs: Arc<RwLock<HashMap<String, BackfillJob>>>,
    /// 產生同一秒內多筆 job id 的遞增序號。
    next_id: Arc<AtomicU64>,
}

/// Backfill admin 的全域記憶體狀態。
pub(super) static BACKFILL_STATE: Lazy<BackfillWebState> = Lazy::new(BackfillWebState::new);

impl BackfillWebState {
    /// 建立空的 job 狀態容器。
    pub(super) fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// 產生新的 job id。
    ///
    /// 格式為 `yyyyMMddHHmmss-seq`，同時保留時間排序資訊與單程序內唯一性。
    pub(super) fn next_job_id(&self) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("{}-{seq}", Local::now().format("%Y%m%d%H%M%S"))
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
