//! Manual backfill web UI and API.
//!
//! 這個模組提供一個輕量的 Web UI 與 JSON API，讓維運人員可以手動觸發
//! 收盤彙總、持股已領股利重算、歷年股利補抓等資料修補工作。所有工作都會
//! 先登記成 job，再由背景 task 執行，呼叫端可用 job API 查詢執行狀態。

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{anyhow, Result};
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use chrono::{Local, NaiveDate};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{
    backfill::dividend, calculation::dividend_record, event::taiwan_stock::closing, logging,
};

/// Manual backfill Web API 共用狀態。
///
/// 狀態目前保存在記憶體中，適合單一程序內的臨時手動維運用途。
#[derive(Clone)]
struct BackfillWebState {
    /// 以 job id 為 key 的回補工作表。
    jobs: Arc<RwLock<HashMap<String, BackfillJob>>>,
    /// 產生同一秒內多筆 job id 的遞增序號。
    next_id: Arc<AtomicU64>,
}

/// Manual backfill 的全域記憶體狀態。
static BACKFILL_STATE: Lazy<BackfillWebState> = Lazy::new(BackfillWebState::new);

impl BackfillWebState {
    /// 建立空的 job 狀態容器。
    fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// 產生新的 job id。
    ///
    /// 格式為 `yyyyMMddHHmmss-seq`，同時保留時間排序資訊與單程序內唯一性。
    fn next_job_id(&self) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("{}-{seq}", Local::now().format("%Y%m%d%H%M%S"))
    }
}

/// Manual backfill job 的執行狀態。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum BackfillJobStatus {
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
    status: BackfillJobStatus,
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

/// 收盤彙總手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
struct ClosingAggregateRequest {
    /// 交易日期，格式必須為 `YYYY-MM-DD`。
    date: String,
}

/// 證券代號類手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
struct SecurityCodeRequest {
    /// 股票、ETF 或其他證券代號。
    security_code: String,
}

/// 建立 job 成功時的 HTTP response body。
#[derive(Debug, Serialize)]
struct StartJobResponse {
    /// 已建立的 job。
    job: BackfillJob,
}

/// API 錯誤回應。
#[derive(Debug, Serialize)]
struct ErrorResponse {
    /// 可讀的錯誤原因。
    error: String,
}

/// 建立 manual backfill 的 Web UI 與 JSON API router。
///
/// 路由包含：
/// - `GET /manual-backfill`：操作頁面。
/// - `GET /api/manual-backfill/jobs`：列出所有 job。
/// - `GET /api/manual-backfill/jobs/{id}`：查詢單一 job。
/// - `POST /api/manual-backfill/*`：建立不同類型的回補 job。
pub fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/manual-backfill") }))
        .route("/manual-backfill", get(index))
        .route("/api/manual-backfill/jobs", get(list_jobs))
        .route("/api/manual-backfill/jobs/{id}", get(get_job))
        .route(
            "/api/manual-backfill/closing-aggregate",
            post(start_closing_aggregate),
        )
        .route(
            "/api/manual-backfill/received-dividend-records",
            post(start_received_dividend_records),
        )
        .route(
            "/api/manual-backfill/historical-dividends",
            post(start_historical_dividends),
        )
        .with_state(BACKFILL_STATE.clone())
}

/// 回傳 manual backfill 操作頁 HTML。
async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// 列出目前程序記憶體中的所有 manual backfill jobs。
async fn list_jobs(State(_state): State<BackfillWebState>) -> Json<Vec<BackfillJob>> {
    Json(list_backfill_jobs().await)
}

/// 依 job id 查詢單一 manual backfill job。
async fn get_job(
    State(_state): State<BackfillWebState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match get_backfill_job(&id).await {
        Some(job) => Json(job).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("job not found: {id}"),
            }),
        )
            .into_response(),
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

/// 建立收盤彙總回補 job 的 HTTP handler。
async fn start_closing_aggregate(
    State(_state): State<BackfillWebState>,
    Json(req): Json<ClosingAggregateRequest>,
) -> impl IntoResponse {
    // 先驗證日期格式，避免背景 job 才因輸入錯誤失敗。
    let date = match NaiveDate::parse_from_str(req.date.trim(), "%Y-%m-%d") {
        Ok(date) => date,
        Err(why) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("date must use YYYY-MM-DD: {why}"),
                }),
            )
                .into_response();
        }
    };

    // 輸入有效時建立背景 job，立即回傳 job 狀態給呼叫端輪詢。
    Json(StartJobResponse {
        job: start_closing_aggregate_job(date).await,
    })
    .into_response()
}

/// 建立持股已領股利回補 job 的 HTTP handler。
async fn start_received_dividend_records(
    State(_state): State<BackfillWebState>,
    Json(req): Json<SecurityCodeRequest>,
) -> impl IntoResponse {
    // 正規化證券代號，確保後續 crawler/database 查詢拿到乾淨輸入。
    let security_code = match normalize_security_code(req.security_code) {
        Ok(security_code) => security_code,
        Err(why) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: why.to_string(),
                }),
            )
                .into_response();
        }
    };
    // 建立背景 job，HTTP request 不等待實際回補流程完成。
    Json(StartJobResponse {
        job: start_received_dividend_records_job(security_code).await,
    })
    .into_response()
}

/// 建立歷年股利回補 job 的 HTTP handler。
async fn start_historical_dividends(
    State(_state): State<BackfillWebState>,
    Json(req): Json<SecurityCodeRequest>,
) -> impl IntoResponse {
    // 歷年股利回補只接受簡單英數證券代號，避免把任意字串帶入外部查詢。
    let security_code = match normalize_security_code(req.security_code) {
        Ok(security_code) => security_code,
        Err(why) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: why.to_string(),
                }),
            )
                .into_response();
        }
    };
    // 建立背景 job，回補結果會更新到 job message。
    Json(StartJobResponse {
        job: start_historical_dividends_job(security_code).await,
    })
    .into_response()
}

/// 建立收盤彙總背景 job。
///
/// Job 會呼叫 `closing::aggregate` 重新彙總指定交易日的收盤資料。
pub(crate) async fn start_closing_aggregate_job(date: NaiveDate) -> BackfillJob {
    start_job(
        BACKFILL_STATE.clone(),
        "closing_aggregate",
        date.to_string(),
        move || async move {
            closing::aggregate(date).await?;
            Ok("closing aggregate backfill completed".to_string())
        },
    )
    .await
}

/// 建立持股已領股利重算背景 job。
///
/// Job 會針對單一證券代號呼叫已領股利紀錄回補流程，完成後將摘要寫入 message。
pub(crate) async fn start_received_dividend_records_job(security_code: String) -> BackfillJob {
    // `input` 用於 job 查詢顯示，closure 則保留原始 security_code 供實際回補使用。
    let input = security_code.clone();
    start_job(
        BACKFILL_STATE.clone(),
        "received_dividend_records",
        input,
        move || async move {
            let summary =
                dividend_record::backfill_received_dividend_records_for_stock(&security_code)
                    .await?;
            Ok(format!(
                "received dividend records backfill completed: holding_count={}, year_count={}, recalculated_count={}",
                summary.holding_count, summary.year_count, summary.recalculated_count
            ))
        },
    )
    .await
}

/// 建立歷年股利補抓背景 job。
///
/// Job 會從 Yahoo 股利頁重新抓取單一證券的歷年配息資料並 upsert 回資料庫。
pub(crate) async fn start_historical_dividends_job(security_code: String) -> BackfillJob {
    // `input` 用於 job 查詢顯示，closure 則保留原始 security_code 供實際回補使用。
    let input = security_code.clone();
    start_job(
        BACKFILL_STATE.clone(),
        "historical_dividends",
        input,
        move || async move {
            let upserted_count =
                dividend::backfill_historical_dividends_for_stock(&security_code).await?;
            Ok(format!(
                "historical dividends backfill completed: upserted_count={upserted_count}"
            ))
        },
    )
    .await
}

/// 建立並啟動一個 manual backfill 背景 job。
///
/// 此 helper 封裝共用流程：建立初始 job、寫入記憶體狀態、spawn 背景 task、
/// 依執行結果更新 job 狀態與 log。
async fn start_job<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    run: F,
) -> BackfillJob
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<String>> + Send + 'static,
{
    // 先建立 running 狀態的 job，讓呼叫端能立即取得 id。
    let job = BackfillJob {
        id: state.next_job_id(),
        kind: kind.to_string(),
        input,
        status: BackfillJobStatus::Running,
        message: "queued".to_string(),
        started_at: Local::now().to_rfc3339(),
        finished_at: None,
    };
    // job_id 用於背景 task 完成時回頭更新同一筆 job。
    let job_id = job.id.clone();
    // task_job 保留啟動 log 需要的資訊，避免 move 掉回傳用的 job。
    let task_job = job.clone();

    // 寫入全域 job 表後再 spawn，確保呼叫端一拿到 id 就查得到。
    state.jobs.write().await.insert(job_id.clone(), job.clone());

    // 背景執行實際回補工作，HTTP/gRPC 呼叫端只負責後續輪詢。
    let jobs = Arc::clone(&state.jobs);
    tokio::spawn(async move {
        logging::info_file_async(format!(
            "manual backfill job started: id={}, kind={}, input={}",
            task_job.id, task_job.kind, task_job.input
        ));

        // 執行呼叫端提供的回補流程，成功時回傳完成訊息，失敗時保留錯誤鏈。
        let result = run().await;
        // 取得寫鎖後更新 job 狀態；鎖只包住狀態更新，避免長時間持鎖執行回補。
        let mut jobs = jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            job.finished_at = Some(Local::now().to_rfc3339());

            match result {
                Ok(message) => {
                    // 成功時標記 succeeded，並把回補摘要提供給 UI/API 顯示。
                    job.status = BackfillJobStatus::Succeeded;
                    job.message = message;
                    logging::info_file_async(format!(
                        "manual backfill job succeeded: id={}, kind={}, input={}, message={}",
                        job.id, job.kind, job.input, job.message
                    ));
                }
                Err(why) => {
                    // 失敗時標記 failed，message 使用 anyhow 的完整錯誤鏈。
                    job.status = BackfillJobStatus::Failed;
                    job.message = format!("{why:#}");
                    logging::error_file_async(format!(
                        "manual backfill job failed: id={}, kind={}, input={}, error={:#}",
                        job.id, job.kind, job.input, why
                    ));
                }
            }
        }
    });

    job
}

/// 正規化並驗證證券代號。
///
/// 此函式會 trim 頭尾空白，拒絕空字串與非 ASCII 英數字元，成功時回傳清理後的代號。
pub(crate) fn normalize_security_code(security_code: String) -> Result<String> {
    // 移除表單或 API 呼叫常見的頭尾空白。
    let security_code = security_code.trim();
    // 空代號無法執行任何回補流程，直接回報輸入錯誤。
    if security_code.is_empty() {
        return Err(anyhow!("security_code is required"));
    }
    // 僅允許英數代號，避免把符號或空白帶入 crawler/database 查詢。
    if !security_code.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(anyhow!(
            "security_code may only contain ASCII letters or numbers"
        ));
    }
    // 回傳擁有權字串，讓後續 async job 可以安全 move 進背景 task。
    Ok(security_code.to_string())
}

/// Manual backfill 單頁操作介面。
const INDEX_HTML: &str = r##"<!doctype html>
<html lang="zh-Hant">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Manual Backfill</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f6f7f9;
      --panel: #ffffff;
      --text: #1d252d;
      --muted: #65717d;
      --line: #dce2e8;
      --accent: #146c63;
      --accent-dark: #0f504a;
      --danger: #b42318;
      --ok: #157347;
      --running: #8a5a00;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      font-size: 15px;
      line-height: 1.5;
    }
    header {
      border-bottom: 1px solid var(--line);
      background: var(--panel);
    }
    .wrap {
      width: min(1120px, calc(100% - 32px));
      margin: 0 auto;
    }
    header .wrap {
      min-height: 72px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
    }
    h1 {
      margin: 0;
      font-size: 24px;
      font-weight: 700;
    }
    main {
      padding: 24px 0 40px;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 16px;
      align-items: start;
    }
    .panel {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 16px;
    }
    h2 {
      margin: 0 0 12px;
      font-size: 18px;
    }
    label {
      display: block;
      color: var(--muted);
      font-size: 13px;
      margin-bottom: 6px;
    }
    input {
      width: 100%;
      height: 40px;
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 0 10px;
      font: inherit;
      background: #fff;
      color: var(--text);
    }
    button {
      width: 100%;
      height: 40px;
      border: 0;
      border-radius: 6px;
      margin-top: 12px;
      background: var(--accent);
      color: #fff;
      font: inherit;
      font-weight: 650;
      cursor: pointer;
    }
    button:hover { background: var(--accent-dark); }
    button:disabled { opacity: .65; cursor: progress; }
    .jobs {
      margin-top: 18px;
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      overflow: hidden;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      table-layout: fixed;
    }
    th, td {
      padding: 10px 12px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: top;
      overflow-wrap: anywhere;
    }
    th {
      color: var(--muted);
      font-size: 13px;
      font-weight: 650;
      background: #fbfcfd;
    }
    tr:last-child td { border-bottom: 0; }
    .status {
      display: inline-flex;
      align-items: center;
      min-height: 24px;
      padding: 2px 8px;
      border-radius: 999px;
      font-size: 13px;
      font-weight: 650;
      background: #eef1f4;
    }
    .running { color: var(--running); }
    .succeeded { color: var(--ok); }
    .failed { color: var(--danger); }
    .toast {
      color: var(--muted);
      min-height: 22px;
      margin-top: 10px;
      overflow-wrap: anywhere;
    }
    @media (max-width: 840px) {
      .grid { grid-template-columns: 1fr; }
      header .wrap { align-items: flex-start; flex-direction: column; padding: 16px 0; }
      th:nth-child(1), td:nth-child(1) { display: none; }
    }
  </style>
</head>
<body>
  <header>
    <div class="wrap">
      <h1>Manual Backfill</h1>
      <div id="summary" class="toast">Loading jobs...</div>
    </div>
  </header>
  <main class="wrap">
    <section class="grid" aria-label="Backfill forms">
      <form class="panel" data-endpoint="/api/manual-backfill/closing-aggregate">
        <h2>Closing Aggregate</h2>
        <label for="closing-date">Trading date</label>
        <input id="closing-date" name="date" type="date" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/received-dividend-records">
        <h2>Received Dividends</h2>
        <label for="received-code">Security code</label>
        <input id="received-code" name="security_code" inputmode="latin" placeholder="0056" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/historical-dividends">
        <h2>Historical Dividends</h2>
        <label for="historical-code">Security code</label>
        <input id="historical-code" name="security_code" inputmode="latin" placeholder="2845" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
    </section>
    <section class="jobs" aria-label="Backfill jobs">
      <table>
        <thead>
          <tr>
            <th style="width: 18%">Job</th>
            <th style="width: 18%">Type</th>
            <th style="width: 14%">Input</th>
            <th style="width: 14%">Status</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody id="jobs-body">
          <tr><td colspan="5">No jobs yet.</td></tr>
        </tbody>
      </table>
    </section>
  </main>
  <script>
    const jobsBody = document.querySelector("#jobs-body");
    const summary = document.querySelector("#summary");

    document.querySelectorAll("form[data-endpoint]").forEach((form) => {
      form.addEventListener("submit", async (event) => {
        event.preventDefault();
        const button = form.querySelector("button");
        const toast = form.querySelector(".toast");
        const data = Object.fromEntries(new FormData(form).entries());
        button.disabled = true;
        toast.textContent = "Starting...";

        try {
          const response = await fetch(form.dataset.endpoint, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(data)
          });
          const body = await response.json();
          if (!response.ok) throw new Error(body.error || "request failed");
          toast.textContent = `Started job ${body.job.id}`;
          await refreshJobs();
        } catch (error) {
          toast.textContent = error.message;
        } finally {
          button.disabled = false;
        }
      });
    });

    async function refreshJobs() {
      try {
        const response = await fetch("/api/manual-backfill/jobs");
        const jobs = await response.json();
        renderJobs(jobs);
        const running = jobs.filter((job) => job.status === "running").length;
        summary.textContent = `${jobs.length} jobs, ${running} running`;
      } catch (error) {
        summary.textContent = error.message;
      }
    }

    function renderJobs(jobs) {
      if (!jobs.length) {
        jobsBody.innerHTML = '<tr><td colspan="5">No jobs yet.</td></tr>';
        return;
      }
      jobsBody.replaceChildren(...jobs.map((job) => {
        const row = document.createElement("tr");
        row.innerHTML = `
          <td>${escapeHtml(job.id)}</td>
          <td>${escapeHtml(job.kind)}</td>
          <td>${escapeHtml(job.input)}</td>
          <td><span class="status ${job.status}">${escapeHtml(job.status)}</span></td>
          <td>${escapeHtml(job.message)}</td>
        `;
        return row;
      }));
    }

    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, (char) => ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#039;"
      }[char]));
    }

    refreshJobs();
    setInterval(refreshJobs, 3000);
  </script>
</body>
</html>"##;
