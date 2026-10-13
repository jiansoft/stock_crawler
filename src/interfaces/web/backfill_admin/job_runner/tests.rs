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
