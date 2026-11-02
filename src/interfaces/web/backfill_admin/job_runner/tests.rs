use super::*;
use tokio::sync::Semaphore;

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

/// 輪詢全域 [`BACKFILL_STATE`]，等指定 job 離開 running 並回傳最後狀態（最多約 60 秒）。
async fn wait_for_global_job(job: BackfillJob) -> BackfillJob {
    for _ in 0..600 {
        let current = super::super::state::get_backfill_job(&job.id)
            .await
            .expect("job 應可查詢");
        if current.status_label() != "running" {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("job {} ({}) did not finish in time", job.id, job.kind);
}

/// 只讀寫資料庫、不連外部網站的回補 job：從入口建立後實際跑完，摘要寫進 message。
/// 用假代號與 1990 年等沒有資料的參數，臨時庫裡不會改到其他測試的資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn database_only_jobs_run_to_completion() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 database_only_jobs_run_to_completion：無資料庫連接");
        return;
    }
    let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("合法日期");

    let received = start_received_dividend_records_job("79979RJ".to_string())
        .await
        .expect("job should start");
    let received = wait_for_global_job(received).await;
    assert_eq!(received.status_label(), "succeeded", "{}", received.message);
    assert_eq!(
        received.message,
        "received dividend records backfill completed: holding_count=0, year_count=0, recalculated_count=0"
    );

    let multiple = start_multiple_dividend_historical_dividends_job(1990)
        .await
        .expect("job should start");
    let multiple = wait_for_global_job(multiple).await;
    assert_eq!(
        multiple.message,
        "multiple dividend historical dividends backfill completed: stock_count=0, detail_count=0",
        "1990 年沒有多次配息股票，不會對外請求"
    );

    let quote_history = start_quote_history_job(
        vec!["79979QH".to_string()],
        date(2024, 2, 1),
        date(2024, 1, 1),
    )
    .await
    .expect("job should start");
    assert_eq!(quote_history.input, "79979QH|2024-02~2024-01");
    let quote_history = wait_for_global_job(quote_history).await;
    assert_eq!(quote_history.status_label(), "failed");
    assert!(
        quote_history.message.contains("起始月份不得晚於結束月份"),
        "起訖顛倒在抓取前就失敗：{}",
        quote_history.message
    );

    let repair = start_annual_total_repair_job()
        .await
        .expect("job should start");
    assert_eq!(repair.input, "all");
    let repair = wait_for_global_job(repair).await;
    assert_eq!(repair.status_label(), "succeeded", "{}", repair.message);
    assert!(
        repair
            .message
            .starts_with("annual total repair completed: "),
        "{}",
        repair.message
    );

    let cagr = start_cagr_job(Some(date(1990, 1, 2)))
        .await
        .expect("job should start");
    assert_eq!(cagr.input, "1990-01-02");
    let cagr = wait_for_global_job(cagr).await;
    assert!(cagr.message.contains("cagr"), "{}", cagr.message);

    let period = start_cagr_period_job(CagrPeriod::Y1)
        .await
        .expect("job should start");
    assert_eq!(period.input, "Y1");
    let period = wait_for_global_job(period).await;
    assert!(period.message.contains("Y1"), "{}", period.message);
}

/// 會連外部網站的回補入口：相同 `(kind, input)` 已有執行中的 job 時，入口在 spawn 之前就拒絕，
/// 確認各入口組出的 kind 與 input（查重鍵）正確，測試本身不會發出任何請求。
#[tokio::test]
async fn network_jobs_reject_duplicate_of_running_job() {
    let date = NaiveDate::from_ymd_opt(1990, 1, 3).expect("合法日期");
    let blockers = [
        ("daily_quotes", "1990-01-03"),
        ("closing_aggregate", "1990-01-03"),
        ("taiwan_stock_index", "1990-01-03"),
        ("historical_dividends", "79979HD"),
    ];
    {
        let mut jobs = BACKFILL_STATE.jobs.write().await;
        for (kind, input) in blockers {
            let mut job = BackfillJob::for_test(&format!("blocker-{kind}"), None);
            job.kind = kind.to_string();
            job.input = input.to_string();
            jobs.insert(job.id.clone(), job);
        }
    }

    let results = [
        start_daily_quotes_job(date).await,
        start_closing_aggregate_job(date).await,
        start_taiwan_stock_index_job(date).await,
        start_historical_dividends_job("79979HD".to_string()).await,
    ];

    {
        let mut jobs = BACKFILL_STATE.jobs.write().await;
        for (kind, _) in blockers {
            jobs.remove(&format!("blocker-{kind}"));
        }
    }
    for ((kind, _), result) in blockers.iter().zip(results) {
        match result {
            Err(StartJobError::DuplicateActiveJob { existing_id }) => {
                assert_eq!(existing_id, format!("blocker-{kind}"));
            }
            other => panic!("{kind} 應因重複被拒絕，實際：{other:?}"),
        }
    }
}
