//! 手動回補管理介面 handler 的測試：建立 job 前的參數驗證、首頁導向與 job 查詢。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::super::dto::INDEX_HTML;
use super::router;

/// SSE 重連先送最新快照；落後時重讀狀態，完成後送出最後事件並關閉。
#[tokio::test]
async fn sse_reports_latest_progress_and_completes_after_lag() {
    use super::super::state::{BackfillJob, BackfillJobStatus, BackfillWebState};
    use axum::response::IntoResponse;
    use futures::StreamExt;
    let state = BackfillWebState::new();
    state
        .jobs
        .write()
        .await
        .insert("sse".into(), BackfillJob::for_test("sse", None));
    let response = super::stream_jobs(state.clone(), Some("sse".into())).into_response();
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
    use super::super::state::{BackfillJob, BackfillWebState};
    use axum::response::IntoResponse;
    use futures::StreamExt;
    let state = BackfillWebState::new();
    let mut list = super::stream_jobs(state.clone(), None)
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
    let mut single = super::stream_jobs(state.clone(), Some("expired".into()))
        .into_response()
        .into_body()
        .into_data_stream();
    single.next().await.unwrap().unwrap();
    state.jobs.write().await.remove("expired");
    state.events.send(()).unwrap();
    let event = single.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&event).contains("event: expired"));
    assert!(single.next().await.is_none());
    let response = super::single_job_events(
        axum::extract::State(state),
        axum::extract::Path("missing".into()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// 送出 JSON body 並取回狀態碼。
async fn post(path: &str, body: &str) -> StatusCode {
    router()
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("request should build"),
        )
        .await
        .expect("router should serve request")
        .status()
}

/// 送出 GET 並取回（狀態碼, response body 字串）。
async fn get(path: &str) -> (StatusCode, String) {
    let response = router()
        .oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn invalid_input_is_rejected_before_any_job_is_created() {
    // 月份格式錯誤（缺少月份、用了日期格式）。
    assert_eq!(
        post(
            "/api/manual-backfill/quote-history",
            r#"{"stock_symbols":"0050","from":"2015","to":"2021-12"}"#
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    // 代號含非英數字元。
    assert_eq!(
        post(
            "/api/manual-backfill/quote-history",
            r#"{"stock_symbols":"0050; DROP","from":"2015-01","to":"2021-12"}"#
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    // 未知的期間代碼。
    assert_eq!(
        post("/api/manual-backfill/cagr-period", r#"{"period":"Y8"}"#).await,
        StatusCode::BAD_REQUEST
    );
    // 公司行動：比例為零、負數或非數字都必須擋下。
    for ratio in ["0", "-4", "1:4"] {
        assert_eq!(
            post(
                "/api/manual-backfill/corporate-action",
                &format!(
                    r#"{{"stock_symbol":"0050","effective_date":"2025-06-18","share_ratio":"{ratio}"}}"#
                )
            )
            .await,
            StatusCode::BAD_REQUEST,
            "share_ratio={ratio} 應被拒絕"
        );
    }

    // CAGR 基準日格式錯誤（留空才是合法的「採用最新交易日」）。
    // 注意 parse_request_date 沿用 chrono 的寬鬆解析，"2026-8-7" 是合法的，
    // 因此這裡用真正無法解析的值。
    assert_eq!(
        post("/api/manual-backfill/cagr", r#"{"date":"not-a-date"}"#).await,
        StatusCode::BAD_REQUEST
    );
}

/// 根路徑導向操作頁，操作頁回傳內建 HTML。
#[tokio::test]
async fn root_redirects_to_the_admin_page() {
    let (status, _) = get("/").await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let (status, body) = get("/manual-backfill").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, INDEX_HTML);
}

/// job 列表永遠是 JSON 陣列（沒有 job 時為空陣列，而不是 404 或 null）。
#[tokio::test]
async fn job_list_is_always_a_json_array() {
    let (status, body) = get("/api/manual-backfill/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let jobs: serde_json::Value = serde_json::from_str(&body).expect("回應應為合法 JSON");
    assert!(jobs.is_array(), "回應應為陣列：{body}");
}

/// 查無 job 時回 404，並在錯誤訊息帶上查詢的 id。
#[tokio::test]
async fn unknown_job_id_returns_not_found() {
    let (status, body) = get("/api/manual-backfill/jobs/no-such-job").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("no-such-job"), "錯誤訊息應帶上 id：{body}");
}

/// 年度欄位需落在 1900~3000，超出範圍在建立 job 前就擋下。
#[tokio::test]
async fn multiple_dividend_backfill_rejects_out_of_range_years() {
    for year in ["1899", "3001", "0", "-1"] {
        assert_eq!(
            post(
                "/api/manual-backfill/multiple-dividend-historical-dividends",
                &format!(r#"{{"year":{year}}}"#)
            )
            .await,
            StatusCode::BAD_REQUEST,
            "year={year} 應被拒絕"
        );
    }
}

/// 代號類端點都會先驗證證券代號才建立 job。
#[tokio::test]
async fn security_code_endpoints_reject_invalid_codes() {
    for endpoint in [
        "/api/manual-backfill/received-dividend-records",
        "/api/manual-backfill/historical-dividends",
    ] {
        for code in ["", "0050; DROP", "23 30"] {
            assert_eq!(
                post(endpoint, &format!(r#"{{"security_code":"{code}"}}"#)).await,
                StatusCode::BAD_REQUEST,
                "{endpoint} security_code={code:?} 應被拒絕"
            );
        }
    }
}

/// 日期類端點都會先驗證日期格式才建立 job。
#[tokio::test]
async fn date_endpoints_reject_malformed_dates() {
    for endpoint in [
        "/api/manual-backfill/daily-quotes",
        "/api/manual-backfill/closing-aggregate",
        "/api/manual-backfill/taiwan-stock-index",
    ] {
        for date in ["", "2026-04", "2026/04/30"] {
            assert_eq!(
                post(endpoint, &format!(r#"{{"date":"{date}"}}"#)).await,
                StatusCode::BAD_REQUEST,
                "{endpoint} date={date:?} 應被拒絕"
            );
        }
    }
}

/// body 缺欄位或非 JSON 時由 axum 的 extractor 擋下，不會進到 handler。
#[tokio::test]
async fn malformed_request_bodies_never_reach_the_handler() {
    // 缺少必填欄位 → 422。
    assert_eq!(
        post("/api/manual-backfill/daily-quotes", "{}").await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 根本不是 JSON → 400。
    assert_eq!(
        post("/api/manual-backfill/daily-quotes", "not json").await,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn every_form_endpoint_in_the_page_has_a_route() {
    // 表單的 data-endpoint 與 router 若不同步，UI 會安靜地送到 404。
    for endpoint in [
        "/api/manual-backfill/daily-quotes",
        "/api/manual-backfill/closing-aggregate",
        "/api/manual-backfill/taiwan-stock-index",
        "/api/manual-backfill/received-dividend-records",
        "/api/manual-backfill/historical-dividends",
        "/api/manual-backfill/multiple-dividend-historical-dividends",
        "/api/manual-backfill/annual-total-repair",
        "/api/manual-backfill/quote-history",
        "/api/manual-backfill/cagr",
        "/api/manual-backfill/cagr-period",
        "/api/manual-backfill/corporate-action",
    ] {
        assert!(
            INDEX_HTML.contains(&format!("data-endpoint=\"{endpoint}\"")),
            "頁面缺少 {endpoint} 的表單"
        );
    }
}

/// 公司行動登錄成功：寫入後回應帶回該股已登錄的全部事件（假代號、固定歷史日期，測完清除）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn corporate_action_is_saved_and_read_back() {
    use crate::infra::database;

    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 corporate_action_is_saved_and_read_back：無資料庫連接");
        return;
    }
    const SYMBOL: &str = "79979";
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM corporate_action WHERE stock_symbol = $1")
            .bind(SYMBOL)
            .execute(database::get_connection())
            .await;
    };
    cleanup().await;
    let response = router()
        .oneshot(
            Request::post("/api/manual-backfill/corporate-action")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"stock_symbol":"{SYMBOL}","effective_date":"2001-06-18","share_ratio":"0.25","note":" 1 拆 4 "}}"#
                )))
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    cleanup().await;
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("回應是 JSON");
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stock_symbol"], SYMBOL);
    assert_eq!(
        body["actions"],
        serde_json::json!([{"effective_date": "2001-06-18", "share_ratio": "0.25", "note": "1 拆 4"}])
    );
}
