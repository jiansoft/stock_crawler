use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use axum::Json;
use axum::response::IntoResponse;
use chrono::{Local, NaiveDate};

use crate::{
    app::backfill::{dividend, quote, quote_history, taiwan_stock_index},
    app::calculation::{cagr, dividend_record},
    app::event::taiwan_stock::closing,
    domain::performance::CagrPeriod,
};

use super::dto::ErrorResponse;
use super::state::{
    BACKFILL_STATE, BackfillJob, BackfillJobStatus, BackfillWebState, MAX_CONCURRENT_JOBS,
};

/// 單一 manual backfill job 的最長執行時間。
///
/// 批次回補（例如多檔股票逐一向 Yahoo 抓歷年股利）可能要跑數十分鐘，
/// 因此上限放寬到一小時；超過即視為異常（例如外部網站掛住、迴圈卡死），
/// 將 job 標記為 failed 並釋放併行名額，避免殭屍 job 永久佔用資源。
/// 資料寫入已採「單一 transaction 原子替換」，中斷只會 rollback，不會留下半套資料。
const JOB_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// 歷史報價回補專用的逾時上限。
///
/// 這類 job 是「代號數 × 月份數」次外部請求，每次之間還刻意間隔 1.2 秒避免
/// 被 TWSE 限流；全部 ETF 補七年約兩萬次請求要跑數小時，套用一小時的預設
/// 逾時只會在半途被砍掉。
const LONG_RUNNING_JOB_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

/// 建立 manual backfill job 失敗的原因。
///
/// 這是「拒絕建立」的錯誤，不是 job 執行失敗——job 執行失敗會反映在
/// job 本身的 `failed` 狀態上。HTTP 與 gRPC 各自把這個 enum 轉成
/// 對應的錯誤碼（409/429、ALREADY_EXISTS/RESOURCE_EXHAUSTED）。
#[derive(Debug)]
pub(crate) enum StartJobError {
    /// 相同 `(kind, input)` 已有一個執行中的 job。
    ///
    /// 兩份相同的回補同時跑會互相刪除/覆寫資料，因此直接拒絕，
    /// 並附上既有 job 的 id 讓呼叫端可以改為輪詢它。
    DuplicateActiveJob {
        /// 既有執行中 job 的 id。
        existing_id: String,
    },
    /// 執行中的 job 總數已達全域上限。
    TooManyActiveJobs {
        /// 目前設定的併行上限，用於錯誤訊息提示。
        max: usize,
    },
}

impl std::fmt::Display for StartJobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartJobError::DuplicateActiveJob { existing_id } => write!(
                f,
                "the same backfill job is already running: id={existing_id}"
            ),
            StartJobError::TooManyActiveJobs { max } => write!(
                f,
                "too many backfill jobs are running (limit: {max}), try again later"
            ),
        }
    }
}

/// 建立各股每日收盤報價背景 job。
///
/// Job 會呼叫 TWSE 與 TPEx 來源重抓上市櫃各股每日開高低收、成交量與本益比等
/// 資料，抓取成功後才在單一資料庫 transaction 內刪除同日舊資料並寫入新資料。
pub(crate) async fn start_daily_quotes_job(date: NaiveDate) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "daily_quotes",
        date.to_string(),
        move || async move {
            // quote::execute 內部採「先抓取、後原子替換」：抓取失敗時完全不動資料庫；
            // 寫入階段的「刪除舊資料 + COPY 新資料」綁在同一個 transaction，
            // 任一步失敗都會 rollback。因此這裡不需要（也不可以）先手動刪除
            // 當日資料——那正是舊版「刪除後抓取失敗，行情永久遺失」的缺陷來源。
            let quote_count = quote::execute(date).await?;
            Ok(format!(
                "daily quotes backfill completed: quote_count={quote_count}"
            ))
        },
    )
    .await
}

/// 建立多次配息股票歷年股利批次回補背景 job。
///
/// Job 會找出指定年度已有季配/半年配資料的股票，逐檔重新抓取 Yahoo 歷年股利，
/// 並把成功處理的股票數與明細 upsert 筆數寫入 job message。
pub(crate) async fn start_multiple_dividend_historical_dividends_job(
    year: i32,
) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "multiple_dividend_historical_dividends",
        year.to_string(),
        move || async move {
            let summary =
                dividend::backfill_historical_dividends_for_multiple_dividend_stocks(year).await?;
            Ok(format!(
                "multiple dividend historical dividends backfill completed: stock_count={}, detail_count={}",
                summary.stock_count, summary.detail_count
            ))
        },
    )
    .await
}

/// 建立收盤彙總背景 job。
///
/// Job 會呼叫 `closing::aggregate` 重新彙總指定交易日的收盤資料。
pub(crate) async fn start_closing_aggregate_job(
    date: NaiveDate,
) -> Result<BackfillJob, StartJobError> {
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

/// 建立台股加權指數背景 job。
///
/// Job 會依指定日期呼叫 TWSE 加權股價指數來源，從回傳的整月資料中篩選出
/// 指定日期的那一筆，跳過快取檢查後 upsert `Index` 資料並更新快取。
pub(crate) async fn start_taiwan_stock_index_job(
    date: NaiveDate,
) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "taiwan_stock_index",
        date.to_string(),
        move || async move {
            let upserted_count = taiwan_stock_index::execute_for_date(date).await?;
            Ok(format!(
                "taiwan stock index backfill completed: upserted_count={upserted_count}"
            ))
        },
    )
    .await
}

/// 建立持股已領股利重算背景 job。
///
/// Job 會針對單一證券代號呼叫已領股利紀錄回補流程，完成後將摘要寫入 message。
pub(crate) async fn start_received_dividend_records_job(
    security_code: String,
) -> Result<BackfillJob, StartJobError> {
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
pub(crate) async fn start_historical_dividends_job(
    security_code: String,
) -> Result<BackfillJob, StartJobError> {
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

/// 建立歷史日報價回補背景 job。
///
/// Job 會對每個「代號 × 月份」呼叫 TWSE 個股月行情，寫入時只補空位
/// （`ON CONFLICT DO NOTHING`），既有資料不覆寫也不刪除，因此可安全重跑。
/// 因請求量大，改用 [`LONG_RUNNING_JOB_TIMEOUT`]。
pub(crate) async fn start_quote_history_job(
    stock_symbols: Vec<String>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<BackfillJob, StartJobError> {
    // input 同時是查重鍵：同一組代號與區間不允許併行兩份。
    let input = format!(
        "{}|{}~{}",
        if stock_symbols.is_empty() {
            "all-etf".to_string()
        } else {
            stock_symbols.join(",")
        },
        from.format("%Y-%m"),
        to.format("%Y-%m")
    );

    start_job_with_timeout(
        BACKFILL_STATE.clone(),
        "quote_history",
        input,
        LONG_RUNNING_JOB_TIMEOUT,
        move || async move {
            // 空清單代表「全部未下市的 00 開頭 ETF／ETN」，於 job 內解析，
            // 讓新掛牌的標的自動納入。
            let symbols = if stock_symbols.is_empty() {
                quote_history::fetch_etf_symbols().await?
            } else {
                stock_symbols
            };
            let summary = quote_history::execute(&symbols, from, to).await?;
            Ok(format!(
                "quote history backfill completed: symbols={}, months_requested={}, months_with_data={}, months_failed={}, quotes_fetched={}, rows_inserted={}",
                symbols.len(),
                summary.months_requested,
                summary.months_with_data,
                summary.months_failed,
                summary.quotes_fetched,
                summary.rows_inserted
            ))
        },
    )
    .await
}

/// 建立指定基準日的 CAGR 重算背景 job。
///
/// `date` 為 `None` 時採用資料庫中最新的交易日。計算只讀既有報價與股利，
/// 不呼叫外部網站；同一 `(基準日, 股票, 期間)` 為冪等的 upsert 覆蓋。
pub(crate) async fn start_cagr_job(date: Option<NaiveDate>) -> Result<BackfillJob, StartJobError> {
    let input = date.map_or_else(|| "latest".to_string(), |value| value.to_string());

    start_job(
        BACKFILL_STATE.clone(),
        "cagr",
        input,
        move || async move {
            let summary = cagr::execute(date).await?;
            Ok(format!(
                "cagr calculation completed: date={:?}, universe={}, periods_calculated={}, periods_skipped={}, rows_written={}, anomaly_symbols={}",
                summary.date,
                summary.universe,
                summary.periods_calculated,
                summary.periods_skipped,
                summary.rows_written,
                summary.anomaly_symbols
            ))
        },
    )
    .await
}

/// 建立單一統計期間的歷史回填背景 job。
///
/// 新增期間（例如 Y7）後專用：掃出「已有計算結果、但缺少該期間」的基準日
/// 逐日補算該期間。待回填的日期可能很多，改用
/// [`LONG_RUNNING_JOB_TIMEOUT`]。
pub(crate) async fn start_cagr_period_job(
    period: CagrPeriod,
) -> Result<BackfillJob, StartJobError> {
    start_job_with_timeout(
        BACKFILL_STATE.clone(),
        "cagr_period",
        period.code().to_string(),
        LONG_RUNNING_JOB_TIMEOUT,
        move || async move {
            let summary = cagr::backfill_period(period).await?;
            Ok(format!(
                "cagr period backfill completed: period={}, dates_pending={}, dates_processed={}, dates_skipped={}, rows_written={}",
                period.code(),
                summary.dates_pending,
                summary.dates_processed,
                summary.dates_skipped,
                summary.rows_written
            ))
        },
    )
    .await
}

/// 建立並啟動一個 manual backfill 背景 job。
///
/// 此 helper 封裝共用流程，依序做四件事：
///
/// 1. **查重**：相同 `(kind, input)` 已有執行中的 job 時拒絕建立，
///    避免兩份相同回補互相刪除/覆寫資料。
/// 2. **併行閘門**：以 semaphore 限制同時執行的 job 總數
///    （[`MAX_CONCURRENT_JOBS`]），滿了直接拒絕而不是排隊。
/// 3. **登記與修剪**：建立 running 狀態的 job 寫入記憶體狀態表，
///    並順手修剪過舊的已完成 job，讓狀態表大小有上限。
/// 4. **背景執行**：spawn task 執行回補（帶 [`JOB_TIMEOUT`] 逾時保護），
///    完成後回頭更新 job 狀態與 log。
///
/// 查重、領名額、登記三步都發生在同一次 `jobs` 寫鎖期間——
/// 這保證兩個並發請求不可能同時通過查重（check-then-act 競態）。
async fn start_job<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    run: F,
) -> Result<BackfillJob, StartJobError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
{
    start_job_with_timeout(state, kind, input, JOB_TIMEOUT, run).await
}

/// 與 [`start_job`] 相同，但可指定逾時上限。
///
/// 逾時是「這個 job 合理跑多久」的宣告，不同工作差異可以很大：抓一天的收盤
/// 報價幾秒就好，補七年的個股月行情要數小時。用同一個值只能遷就最慢的那個，
/// 反而讓真正卡住的 job 拖著名額不放。
async fn start_job_with_timeout<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    timeout: Duration,
    run: F,
) -> Result<BackfillJob, StartJobError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
{
    // 取得寫鎖。從這裡到 drop(jobs_guard) 之間的檢查與登記是原子的：
    // 其他請求必須等鎖釋放才能做自己的查重，因此不會有兩個相同 job 同時通過。
    let mut jobs_guard = state.jobs.write().await;

    // 防護 1：相同 (kind, input) 只允許一個執行中的 job。
    if let Some(existing_id) = BackfillWebState::find_active_job(&jobs_guard, kind, &input) {
        return Err(StartJobError::DuplicateActiveJob { existing_id });
    }

    // 防護 2：全域併行上限。try_acquire 立即返回，不會讓 HTTP request 掛著等。
    // permit 是 RAII 憑證，稍後 move 進背景 task；task 結束（成功、失敗、逾時、
    // panic）時 permit 被 drop，名額自動歸還。
    let permit = state
        .try_acquire_job_slot()
        .ok_or(StartJobError::TooManyActiveJobs {
            max: MAX_CONCURRENT_JOBS,
        })?;

    // 通過兩道防護後才建立 running 狀態的 job，讓呼叫端能立即取得 id。
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
    jobs_guard.insert(job_id.clone(), job.clone());
    // 防護 3：趁著已持有寫鎖，修剪超過保留上限的已完成 job，
    // 讓長時間運行的程序記憶體不會被歷史 job 無限撐大。
    BackfillWebState::prune_finished_jobs(&mut jobs_guard);
    // 登記完成，儘早釋放寫鎖；背景 task 的實際回補不應在持鎖狀態下執行。
    drop(jobs_guard);

    // 背景執行實際回補工作，HTTP/gRPC 呼叫端只負責後續輪詢。
    let jobs = Arc::clone(&state.jobs);
    // 在 spawn 前就登記，而不是進入 async block 後才登記。
    // 原因是 tokio::spawn 只把 future 放進排程佇列，不保證立刻 poll；若 OS 訊號恰好在
    // 兩者之間抵達，main 可能看到 active=0 而提早退出。先取得 guard 可關閉這個競態窗口。
    let operation_guard = crate::core::shutdown::BACKGROUND_OPERATIONS.begin();
    tokio::spawn(async move {
        // 把 guard 與 permit move 進 task 並綁在整個 scope。以下 run().await 不論成功、
        // 錯誤、逾時或 panic unwind，Rust 都會 drop 這兩個 RAII 物件：
        // 關機 counter 不會永久卡住，併行名額也一定會歸還。
        let _operation_guard = operation_guard;
        let _job_slot_permit = permit;
        tracing::info!(
            "manual backfill job started: id={}, kind={}, input={}",
            task_job.id,
            task_job.kind,
            task_job.input
        );

        // 防護 4：逾時保護。tokio::time::timeout 在時限內回傳 Ok(內層結果)；
        // 超時則直接 drop 掉還在執行的 future（正在進行的資料庫 transaction
        // 會因連線歸還而自動 rollback，不會留下半套資料）並回傳 Err。
        let result = match tokio::time::timeout(timeout, run()).await {
            Ok(result) => result,
            Err(_elapsed) => Err(anyhow!("job timed out after {} seconds", timeout.as_secs())),
        };
        // 取得寫鎖後更新 job 狀態；鎖只包住狀態更新，避免長時間持鎖執行回補。
        let mut jobs = jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            job.finished_at = Some(Local::now().to_rfc3339());

            match result {
                Ok(message) => {
                    // 成功時標記 succeeded，並把回補摘要提供給 UI/API 顯示。
                    job.status = BackfillJobStatus::Succeeded;
                    job.message = message;
                    tracing::info!(
                        "manual backfill job succeeded: id={}, kind={}, input={}, message={}",
                        job.id,
                        job.kind,
                        job.input,
                        job.message
                    );
                }
                Err(why) => {
                    // 失敗時標記 failed，message 使用 anyhow 的完整錯誤鏈。
                    job.status = BackfillJobStatus::Failed;
                    job.message = format!("{why:#}");
                    tracing::error!(
                        "manual backfill job failed: id={}, kind={}, input={}, error={:#}",
                        job.id,
                        job.kind,
                        job.input,
                        why
                    );
                }
            }
        }
    });

    Ok(job)
}

/// 將 [`StartJobError`] 轉成一致的 HTTP 錯誤回應。
///
/// - 重複 job → `409 Conflict`：同一份工作已在執行，訊息附上既有 job id。
/// - 併行已滿 → `429 Too Many Requests`：請呼叫端稍後重試。
pub(super) fn start_job_error_response(err: StartJobError) -> axum::response::Response {
    let status = match err {
        StartJobError::DuplicateActiveJob { .. } => axum::http::StatusCode::CONFLICT,
        StartJobError::TooManyActiveJobs { .. } => axum::http::StatusCode::TOO_MANY_REQUESTS,
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
        .into_response()
}

/// 解析 HTTP request 的日期欄位，格式錯誤時回傳一致的 400 response。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_date(date: &str) -> Result<NaiveDate, axum::response::Response> {
    NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("date must use YYYY-MM-DD: {why}"),
            }),
        )
            .into_response()
    })
}

/// 解析 HTTP request 的月份欄位（`YYYY-MM`），回傳該月第一天。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_month(month: &str) -> Result<NaiveDate, axum::response::Response> {
    // `<input type="month">` 送出的是 `YYYY-MM`，補上第一天才能解析成日期。
    NaiveDate::parse_from_str(&format!("{}-01", month.trim()), "%Y-%m-%d").map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("month must use YYYY-MM: {why}"),
            }),
        )
            .into_response()
    })
}

/// 解析 HTTP request 的期間代碼欄位。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_period(period: &str) -> Result<CagrPeriod, axum::response::Response> {
    CagrPeriod::from_code(period.trim()).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("unknown period: {period}"),
            }),
        )
            .into_response()
    })
}

/// 解析公司行動的股數變動比例。
///
/// 必須大於零：`0` 會讓持股歸零、負數毫無意義，兩者都只會產生錯得離譜的
/// 報酬率。上界取 1000，擋掉把「1:4」整串貼進來之類的輸入錯誤。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_share_ratio(
    raw: &str,
) -> Result<rust_decimal::Decimal, axum::response::Response> {
    let reject = |message: String| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: message }),
        )
            .into_response()
    };

    let ratio = rust_decimal::Decimal::from_str_exact(raw.trim())
        .map_err(|why| reject(format!("share_ratio must be a decimal number: {why}")))?;
    if ratio <= rust_decimal::Decimal::ZERO {
        return Err(reject("share_ratio must be greater than zero".to_string()));
    }
    if ratio > rust_decimal::Decimal::from(1000) {
        return Err(reject("share_ratio looks implausible (> 1000)".to_string()));
    }
    Ok(ratio)
}

/// 解析以逗號、空白或換行分隔的代號清單；空字串回傳空 `Vec`。
///
/// 空清單在呼叫端有明確語意（全部 ETF），因此不視為錯誤；
/// 但只要有填就逐一驗證，避免把打錯的代號送去打外部 API。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_symbol_list(
    raw: &str,
) -> Result<Vec<String>, axum::response::Response> {
    let mut symbols = Vec::new();
    for token in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.trim().is_empty() {
            continue;
        }
        symbols.push(parse_request_security_code(token.to_string())?);
    }
    symbols.sort_unstable();
    symbols.dedup();
    Ok(symbols)
}

/// 解析 HTTP request 的證券代號欄位，格式錯誤時回傳一致的 400 response。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_security_code(
    security_code: String,
) -> Result<String, axum::response::Response> {
    normalize_security_code(security_code).map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: why.to_string(),
            }),
        )
            .into_response()
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use rust_decimal_macros::dec;
    use tokio::sync::Semaphore;

    /// 取出解析失敗時回傳的 HTTP 狀態碼。
    fn status_of<T>(result: Result<T, axum::response::Response>) -> StatusCode {
        result
            .err()
            .map(|response| response.status())
            .expect("應為解析失敗")
    }

    /// 日期需為 `YYYY-MM-DD`；解析失敗一律回 400。
    #[test]
    fn request_date_accepts_iso_dates_and_rejects_the_rest() {
        assert_eq!(
            parse_request_date(" 2026-04-30 ").expect("合法日期"),
            NaiveDate::from_ymd_opt(2026, 4, 30).expect("測試日期應合法")
        );
        for raw in ["", "2026-04", "2026/04/30", "not-a-date", "2026-02-30"] {
            assert_eq!(status_of(parse_request_date(raw)), StatusCode::BAD_REQUEST);
        }
    }

    /// `<input type="month">` 送出的 `YYYY-MM` 應解析成該月一日。
    #[test]
    fn request_month_resolves_to_the_first_day() {
        assert_eq!(
            parse_request_month(" 2015-01 ").expect("合法月份"),
            NaiveDate::from_ymd_opt(2015, 1, 1).expect("測試日期應合法")
        );
        for raw in ["", "2015", "2015-13", "2015-01-01"] {
            assert_eq!(status_of(parse_request_month(raw)), StatusCode::BAD_REQUEST);
        }
    }

    /// 期間代碼必須對得上 `CagrPeriod`，未知代碼在建立 job 前就擋下。
    #[test]
    fn request_period_only_accepts_known_codes() {
        let period = parse_request_period(" Y1 ").expect("Y1 應為合法期間");
        assert_eq!(period.code(), "Y1");
        for raw in ["", "Y99", "1Y"] {
            assert_eq!(
                status_of(parse_request_period(raw)),
                StatusCode::BAD_REQUEST
            );
        }
    }

    /// 股數變動比例必須為正數且不得離譜；邊界值 1000 仍屬合法。
    #[test]
    fn request_share_ratio_guards_against_nonsense_values() {
        assert_eq!(parse_request_share_ratio(" 4 ").expect("正整數"), dec!(4));
        assert_eq!(
            parse_request_share_ratio("0.5").expect("小數比例"),
            dec!(0.5)
        );
        assert_eq!(
            parse_request_share_ratio("1000").expect("上界值應合法"),
            dec!(1000)
        );
        // 0 會讓持股歸零、負數無意義、`1:4` 是把整串貼進來的常見錯誤。
        for raw in ["0", "-4", "1:4", "", "1000.1"] {
            assert_eq!(
                status_of(parse_request_share_ratio(raw)),
                StatusCode::BAD_REQUEST,
                "share_ratio={raw} 應被拒絕"
            );
        }
    }

    /// 代號清單支援逗號／空白／換行混用，並排序去重；空字串代表「全部 ETF」。
    #[test]
    fn request_symbol_list_normalizes_separators_and_duplicates() {
        assert_eq!(
            parse_request_symbol_list("0056, 0050\n0056\t2330").expect("合法清單"),
            vec!["0050".to_string(), "0056".to_string(), "2330".to_string()]
        );
        assert!(
            parse_request_symbol_list("  \n ")
                .expect("空白視為空清單")
                .is_empty()
        );
        // 只要有一個代號非法，整份清單就該被拒絕，不能只丟掉那一個。
        assert_eq!(
            status_of(parse_request_symbol_list("0050, 00;50")),
            StatusCode::BAD_REQUEST
        );
    }

    /// 證券代號只接受 ASCII 英數，並去除頭尾空白。
    #[test]
    fn security_code_is_trimmed_and_restricted_to_ascii_alphanumerics() {
        assert_eq!(
            normalize_security_code(" 2330 ".to_string()).expect("合法代號"),
            "2330"
        );
        // trim 只作用於頭尾，中間的空白仍屬非法字元。
        assert_eq!(
            normalize_security_code("2330\n".to_string()).expect("尾端換行應被 trim"),
            "2330"
        );
        for raw in ["", "   ", "23 30", "0050;DROP", "台積電"] {
            assert!(
                normalize_security_code(raw.to_string()).is_err(),
                "security_code={raw:?} 應被拒絕"
            );
        }
        assert_eq!(
            status_of(parse_request_security_code("0050;".to_string())),
            StatusCode::BAD_REQUEST
        );
    }

    /// 建立被拒絕的兩種原因各自對應固定的 HTTP 狀態碼。
    #[test]
    fn start_job_errors_map_to_conflict_and_too_many_requests() {
        let duplicate = StartJobError::DuplicateActiveJob {
            existing_id: "20260815000000-1".to_string(),
        };
        assert!(duplicate.to_string().contains("20260815000000-1"));
        assert_eq!(
            start_job_error_response(duplicate).status(),
            StatusCode::CONFLICT
        );

        let too_many = StartJobError::TooManyActiveJobs {
            max: MAX_CONCURRENT_JOBS,
        };
        assert!(
            too_many
                .to_string()
                .contains(&MAX_CONCURRENT_JOBS.to_string())
        );
        assert_eq!(
            start_job_error_response(too_many).status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    /// job 失敗時狀態轉為 failed，且 message 保留完整的 anyhow 錯誤鏈。
    #[tokio::test]
    async fn failed_job_records_the_error_chain() {
        let state = BackfillWebState::new();
        let job = start_job(
            state.clone(),
            "test_failure",
            "input".to_string(),
            || async { Err(anyhow!("外層失敗").context("回補流程中止")) },
        )
        .await
        .expect("job should start");

        wait_until_finished(&state, &job.id).await;

        let finished = state
            .jobs
            .read()
            .await
            .get(&job.id)
            .cloned()
            .expect("job should remain queryable");
        assert_eq!(finished.status_label(), "failed");
        assert!(
            finished.message.contains("回補流程中止") && finished.message.contains("外層失敗"),
            "message 應包含完整錯誤鏈：{}",
            finished.message
        );
        assert!(finished.finished_at.is_some());
    }

    /// job 成功時把回補摘要寫進 message，供 UI 直接顯示。
    #[tokio::test]
    async fn succeeded_job_keeps_the_summary_message() {
        let state = BackfillWebState::new();
        let job = start_job(
            state.clone(),
            "test_success",
            "input".to_string(),
            || async { Ok("rows_inserted=42".to_string()) },
        )
        .await
        .expect("job should start");
        // 剛建立時一律是 running / queued，呼叫端可立刻拿 id 去輪詢。
        assert_eq!(job.status_label(), "running");
        assert_eq!(job.message, "queued");

        wait_until_finished(&state, &job.id).await;

        let finished = state
            .jobs
            .read()
            .await
            .get(&job.id)
            .cloned()
            .expect("job should remain queryable");
        assert_eq!(finished.status_label(), "succeeded");
        assert_eq!(finished.message, "rows_inserted=42");
    }

    /// 輪詢等待指定 job 離開 running 狀態（最多約 5 秒）。
    async fn wait_until_finished(state: &BackfillWebState, id: &str) {
        for _ in 0..500 {
            let finished = state
                .jobs
                .read()
                .await
                .get(id)
                .map(|job| job.status_label() != "running")
                .unwrap_or(false);
            if finished {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job {id} did not finish in time");
    }

    /// 驗證相同 (kind, input) 的 job 執行期間會被拒絕，結束後可再啟動。
    #[tokio::test]
    async fn duplicate_active_job_is_rejected() {
        let state = BackfillWebState::new();
        // gate 讓第一個 job 停在執行中，直到測試主動放行；
        // 用 Semaphore(0) 而不是 Notify，是因為 permit 可以累積，
        // 不會因 job 尚未開始等待就遺失喚醒訊號。
        let gate = Arc::new(Semaphore::new(0));

        let g = Arc::clone(&gate);
        let first = start_job(
            state.clone(),
            "test_dup",
            "2026-01-01".to_string(),
            move || {
                async move {
                    // forget()：取得 permit 後不歸還，確保一張 permit 只放行一個 job。
                    g.acquire().await.expect("gate should stay open").forget();
                    Ok("done".to_string())
                }
            },
        )
        .await
        .expect("first job should start");

        // 相同 kind + input 且第一個 job 仍在執行 → 應被拒絕並回報既有 job id。
        let err = start_job(
            state.clone(),
            "test_dup",
            "2026-01-01".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect_err("duplicate job should be rejected");
        match err {
            StartJobError::DuplicateActiveJob { existing_id } => {
                assert_eq!(existing_id, first.id);
            }
            other => panic!("expected DuplicateActiveJob, got {other:?}"),
        }

        // 不同 input 不算重複，可以並行。
        let second = start_job(
            state.clone(),
            "test_dup",
            "2026-01-02".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect("different input should start");

        // 放行第一個 job，等兩個都結束。
        gate.add_permits(1);
        wait_until_finished(&state, &first.id).await;
        wait_until_finished(&state, &second.id).await;

        // 第一個 job 已結束（不再是 active），相同 input 應可重新啟動。
        let again = start_job(
            state.clone(),
            "test_dup",
            "2026-01-01".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect("same input should start after previous job finished");
        wait_until_finished(&state, &again.id).await;
    }

    /// 驗證全域併行上限：滿載時拒絕新 job，名額釋放後恢復受理。
    #[tokio::test]
    async fn too_many_active_jobs_is_rejected() {
        let state = BackfillWebState::new();
        let gate = Arc::new(Semaphore::new(0));

        // 佔滿所有併行名額；每個 job 都停在 gate 上等待放行。
        let mut ids = Vec::new();
        for i in 0..MAX_CONCURRENT_JOBS {
            let g = Arc::clone(&gate);
            let job = start_job(
                state.clone(),
                "test_limit",
                format!("input-{i}"),
                move || async move {
                    g.acquire().await.expect("gate should stay open").forget();
                    Ok("done".to_string())
                },
            )
            .await
            .expect("job within limit should start");
            ids.push(job.id);
        }

        // 第 MAX_CONCURRENT_JOBS + 1 個 job → 名額已滿，應被拒絕。
        let err = start_job(
            state.clone(),
            "test_limit",
            "input-overflow".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect_err("job beyond limit should be rejected");
        assert!(matches!(err, StartJobError::TooManyActiveJobs { .. }));

        // 一次放行所有 job，等全部結束（permit 會被 RAII 歸還）。
        gate.add_permits(MAX_CONCURRENT_JOBS);
        for id in &ids {
            wait_until_finished(&state, id).await;
        }

        // 名額釋放後應可再受理新 job。
        let job = start_job(
            state.clone(),
            "test_limit",
            "input-after".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect("job should start after slots are released");
        wait_until_finished(&state, &job.id).await;
    }

    /// 驗證逾時保護：超過 JOB_TIMEOUT 的 job 會被標記為 failed。
    ///
    /// `start_paused = true` 讓 Tokio 的虛擬時鐘在所有 task 閒置時自動快轉，
    /// 因此測試不需要真的等一小時。
    #[tokio::test(start_paused = true)]
    async fn job_times_out_and_is_marked_failed() {
        let state = BackfillWebState::new();

        // 這個 job 永遠不會自己結束（pending future），只能靠逾時機制收掉。
        let job = start_job(
            state.clone(),
            "test_timeout",
            "input".to_string(),
            || async {
                std::future::pending::<()>().await;
                Ok(String::new())
            },
        )
        .await
        .expect("job should start");

        // 直接把虛擬時鐘推過逾時點，再輪詢等待背景 task 把狀態寫回。
        tokio::time::sleep(JOB_TIMEOUT + Duration::from_secs(1)).await;
        wait_until_finished(&state, &job.id).await;

        let finished = state
            .jobs
            .read()
            .await
            .get(&job.id)
            .cloned()
            .expect("job should remain queryable");
        assert_eq!(finished.status_label(), "failed");
        assert!(
            finished.message.contains("timed out"),
            "message should mention timeout: {}",
            finished.message
        );
    }
}
