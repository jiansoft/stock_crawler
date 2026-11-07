//! 回補 job 進度的 SSE 串流：重連先送快照、落後時重讀、完成與過期事件。

use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;

use crate::interfaces::web::backfill_admin::handlers::{single_job_events, stream_jobs};
use crate::interfaces::web::backfill_admin::state::{
    BackfillJob, BackfillJobStatus, BackfillWebState,
};

/// SSE 重連先送最新快照；落後時重讀狀態，完成後送出最後事件並關閉。
#[tokio::test]
async fn sse_reports_latest_progress_and_completes_after_lag() {
    let state = BackfillWebState::new();
    state
        .jobs
        .write()
        .await
        .insert("sse".into(), BackfillJob::for_test("sse", None));
    let response = stream_jobs(state.clone(), Some("sse".into())).into_response();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body().into_data_stream();
    let first = body.next().await.unwrap().unwrap();
    let first = String::from_utf8_lossy(&first);
    assert!(first.contains("event: progress") && first.contains("\"progress_percent\":0"));
    {
        let mut jobs = state.jobs.write().await;
        let job = jobs.get_mut("sse").unwrap();
        job.progress_percent = 50;
    }
    state.events.send(()).unwrap();
    let update = body.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&update).contains("\"progress_percent\":50"));
    {
        let mut jobs = state.jobs.write().await;
        let job = jobs.get_mut("sse").unwrap();
        job.progress_percent = 100;
        job.status = BackfillJobStatus::Succeeded;
        job.finished_at = Some("2026-10-24T12:00:00+08:00".into());
    }
    // 超過 broadcast 容量後，仍須取得最後結果。
    for _ in 0..100 {
        state.events.send(()).unwrap();
    }
    let last = body.next().await.unwrap().unwrap();
    let last = String::from_utf8_lossy(&last);
    assert!(last.contains("event: complete") && last.contains("\"progress_percent\":100"));
    assert!(body.next().await.is_none());
}

#[tokio::test]
async fn sse_list_snapshot_and_expired_job() {
    let state = BackfillWebState::new();
    let mut list = stream_jobs(state.clone(), None)
        .into_response()
        .into_body()
        .into_data_stream();
    let first = list.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("data: []"));
    state
        .jobs
        .write()
        .await
        .insert("expired".into(), BackfillJob::for_test("expired", None));
    let mut single = stream_jobs(state.clone(), Some("expired".into()))
        .into_response()
        .into_body()
        .into_data_stream();
    single.next().await.unwrap().unwrap();
    state.jobs.write().await.remove("expired");
    state.events.send(()).unwrap();
    let event = single.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&event).contains("event: expired"));
    assert!(single.next().await.is_none());
    let response = single_job_events(
        axum::extract::State(state),
        axum::extract::Path("missing".into()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
