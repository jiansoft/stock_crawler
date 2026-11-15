//! 重運算租約的測試（會員系統計畫 2.3「重運算租約規格」④）。
//!
//! 中介層行為以假的回測 handler 驗證（逾時長度、名額可注入，不必真的等 30 秒）；
//! 路由涵蓋以 OpenAPI 列舉所有含 `/backtest` 的路徑。

use std::path::Path;
use std::time::Duration;

use axum::{Router, body::Body, extract::Query, http::Request, middleware, routing::get};
use serde::Deserialize;
use tower::ServiceExt;

use super::*;

const MEMBER_ACCOUNT: &str = "__lease_member__";

/// 測試期間設定環境變數，結束（含 panic）時還原。`--test-threads=1` 下沒有競爭。
struct EnvGuard(Vec<(&'static str, Option<String>)>);

impl EnvGuard {
    fn set(values: &[(&'static str, Option<&str>)]) -> Self {
        let saved = values
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect();
        for (name, value) in values {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        Self(saved)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}

#[derive(Deserialize)]
struct FakeParams {
    sleep_ms: Option<u64>,
    invalid: Option<bool>,
    slow_sql: Option<bool>,
}

/// 假的回測 handler：可睡一段時間、回 422，或跑一句慢查詢。
async fn fake_backtest(Query(params): Query<FakeParams>) -> Response {
    if params.invalid.unwrap_or(false) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "參數不合法");
    }
    if params.slow_sql.unwrap_or(false) {
        let _ = sqlx::query("SELECT pg_sleep(30) /* lease-statement-timeout-probe */")
            .execute(database::get_connection())
            .await;
    }
    if let Some(ms) = params.sleep_ms {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    error_response(StatusCode::OK, "ok")
}

fn fake_router(state: LeaseState) -> Router {
    Router::new()
        .route("/x/backtest", get(fake_backtest))
        .layer(middleware::from_fn_with_state(state, require_lease))
}

fn state(gate: Arc<AnalysisGate>, timeout: Duration) -> LeaseState {
    LeaseState {
        gate,
        timeout,
        budget: QueryBudget::General,
    }
}

fn request(query: &str, lease: Option<&LeaseHeaders>) -> Request<Body> {
    let mut builder = Request::get(format!("/x/backtest{query}"));
    if let Some(lease) = lease {
        builder = builder
            .header(LEASE_ID_HEADER, &lease.id)
            .header(LEASE_SECRET_HEADER, &lease.secret)
            .header(LEASE_MEMBER_HEADER, lease.member_id.to_string());
    }
    builder.body(Body::empty()).expect("request should build")
}

async fn send(router: &Router, query: &str, lease: Option<&LeaseHeaders>) -> Response {
    router
        .clone()
        .oneshot(request(query, lease))
        .await
        .expect("router should serve request")
}

/// 建立測試會員；回傳會員編號。
async fn test_member(pool: &sqlx::PgPool) -> i64 {
    sqlx::query(r#"DELETE FROM "Member" WHERE "Account" = $1"#)
        .bind(MEMBER_ACCOUNT)
        .execute(pool)
        .await
        .expect("清除殘留測試會員");
    sqlx::query_scalar(
        r#"INSERT INTO "Member" ("Account", "Name", household_id) VALUES ($1, $1, 1) RETURNING "Id""#,
    )
    .bind(MEMBER_ACCOUNT)
    .fetch_one(pool)
    .await
    .expect("建立測試會員")
}

/// 核發一張租約（模擬 stock_go）：回傳標頭內容。
async fn issue(pool: &sqlx::PgPool, member_id: i64, seed: &str) -> LeaseHeaders {
    let (id, secret): (String, String) = sqlx::query_as(
        r#"WITH s AS (SELECT decode(md5($2) || md5($2 || 'x'), 'hex') AS raw)
           INSERT INTO member_analysis_lease (id, member_id, request_id, secret_hash, source, expires_at)
           SELECT gen_random_uuid(), $1, gen_random_uuid()::text, sha256(s.raw), 'mcp', now() + interval '60 seconds'
           FROM s
           RETURNING id::text, (SELECT rtrim(translate(encode(raw, 'base64'), '+/', '-_'), '=') FROM s)"#,
    )
    .bind(member_id)
    .bind(seed)
    .fetch_one(pool)
    .await
    .expect("核發測試租約");
    LeaseHeaders {
        id,
        secret,
        member_id,
    }
}

async fn lease_times(pool: &sqlx::PgPool, id: &str) -> (bool, bool) {
    sqlx::query_as(
        "SELECT started_at IS NOT NULL, finished_at IS NOT NULL FROM member_analysis_lease WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("讀取租約")
}

async fn cleanup(pool: &sqlx::PgPool) {
    // 租約依外來鍵隨會員刪除
    sqlx::query(r#"DELETE FROM "Member" WHERE "Account" = $1"#)
        .bind(MEMBER_ACCOUNT)
        .execute(pool)
        .await
        .expect("清除測試會員");
}

async fn pool_or_skip(test: &str) -> Option<&'static sqlx::PgPool> {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return None;
    }
    Some(pool)
}

/// 標頭解析：三個都沒帶為 Absent；只帶一部分或格式錯誤為 Malformed。
#[test]
fn parse_lease_headers() {
    let mut headers = HeaderMap::new();
    assert_eq!(parse_lease(&headers), LeaseInput::Absent);
    headers.insert(
        LEASE_ID_HEADER,
        HeaderValue::from_static("0193a7b2-0000-7000-8000-000000000001"),
    );
    assert_eq!(parse_lease(&headers), LeaseInput::Malformed);
    headers.insert(
        LEASE_SECRET_HEADER,
        HeaderValue::from_static("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
    );
    headers.insert(LEASE_MEMBER_HEADER, HeaderValue::from_static("2"));
    assert!(matches!(parse_lease(&headers), LeaseInput::Present(_)));
    headers.insert(LEASE_MEMBER_HEADER, HeaderValue::from_static("0"));
    assert_eq!(parse_lease(&headers), LeaseInput::Malformed);
    headers.insert(LEASE_MEMBER_HEADER, HeaderValue::from_static("2"));
    headers.insert(LEASE_SECRET_HEADER, HeaderValue::from_static("short"));
    assert_eq!(parse_lease(&headers), LeaseInput::Malformed);
    headers.insert(
        LEASE_SECRET_HEADER,
        HeaderValue::from_static("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
    );
    headers.insert(LEASE_ID_HEADER, HeaderValue::from_static("not-a-uuid"));
    assert_eq!(parse_lease(&headers), LeaseInput::Malformed);
}

/// 路由涵蓋：OpenAPI 裡每個含 /backtest 的路徑，在嚴格模式下沒帶租約都回 403 lease_invalid；
/// 新增回測端點而沒有掛租約 middleware 時這個測試失敗。
#[tokio::test]
async fn every_backtest_route_requires_lease() {
    use utoipa::OpenApi;
    let _env = EnvGuard::set(&[
        ("DATA_API_KEY", Some("lease-route-market-key")),
        ("DATA_API_STRICT_SCOPE", Some("true")),
    ]);
    let paths: Vec<String> = super::super::openapi::ApiDoc::openapi()
        .paths
        .paths
        .keys()
        .filter(|path| path.contains("/backtest"))
        .cloned()
        .collect();
    assert_eq!(
        paths.len(),
        6,
        "回測端點數有變，三邊（Rust、MCP、stock_go）要一起登記：{paths:?}"
    );
    for path in paths {
        let response = super::super::router()
            .oneshot(
                Request::get(path.as_str())
                    .header("Authorization", "Bearer lease-route-market-key")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{path} 沒有要求租約"
        );
    }
}

/// 回測模組不得自行 tokio::spawn／spawn_blocking：查詢預算不會被繼承、逾時也停不下來。
/// 需要另開 task 時一律經過 lease.rs 的包裝（套用查詢預算與名額）。
#[test]
fn backtest_engine_does_not_spawn_tasks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/interfaces/web/data_api/handlers/screen_backtest");
    let mut offenders = Vec::new();
    let mut stack = vec![root.clone()];
    let backtest_file = root.with_extension("rs");
    let mut files = vec![backtest_file];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("讀取回測模組目錄") {
            let path = entry.expect("目錄項目").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    for file in files {
        let source = std::fs::read_to_string(&file).expect("讀取原始碼");
        if source.contains("tokio::spawn") || source.contains("spawn_blocking") {
            offenders.push(file.display().to_string());
        }
    }
    assert!(offenders.is_empty(), "回測模組自行開 task：{offenders:?}");
}

/// 同一張租約送兩次，第二次 403；密鑰錯一個字元 403；成功的請求寫 finished_at、歸還名額。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn lease_is_single_use_and_secret_must_match() {
    let Some(pool) = pool_or_skip("lease_is_single_use_and_secret_must_match").await else {
        return;
    };
    let member = test_member(pool).await;
    let gate = Arc::new(AnalysisGate::new(1, 0));
    let router = fake_router(state(Arc::clone(&gate), Duration::from_secs(5)));

    let lease = issue(pool, member, "single-use").await;
    assert_eq!(
        send(&router, "", Some(&lease)).await.status(),
        StatusCode::OK
    );
    assert_eq!(lease_times(pool, &lease.id).await, (true, true));
    assert_eq!(gate.available(), 1);
    assert_eq!(
        send(&router, "", Some(&lease)).await.status(),
        StatusCode::FORBIDDEN
    );

    let other = issue(pool, member, "wrong-secret").await;
    let mut tampered = other.clone();
    let first = if tampered.secret.starts_with('A') {
        "B"
    } else {
        "A"
    };
    tampered.secret.replace_range(0..1, first);
    assert_eq!(
        send(&router, "", Some(&tampered)).await.status(),
        StatusCode::FORBIDDEN
    );
    // 會員不符也一樣
    let mut wrong_member = other.clone();
    wrong_member.member_id = member + 1;
    assert_eq!(
        send(&router, "", Some(&wrong_member)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(lease_times(pool, &other.id).await, (false, false));
    cleanup(pool).await;
}

/// 參數驗證失敗（422）不消耗租約，期限內可再用；嚴格模式下沒帶租約 403，相容模式照舊放行。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn validation_failure_releases_lease_and_strict_requires_it() {
    let Some(pool) = pool_or_skip("validation_failure_releases_lease_and_strict_requires_it").await
    else {
        return;
    };
    let member = test_member(pool).await;
    let router = fake_router(state(
        Arc::new(AnalysisGate::new(1, 0)),
        Duration::from_secs(5),
    ));
    let lease = issue(pool, member, "release").await;
    assert_eq!(
        send(&router, "?invalid=true", Some(&lease)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(lease_times(pool, &lease.id).await, (false, false));
    assert_eq!(
        send(&router, "", Some(&lease)).await.status(),
        StatusCode::OK
    );

    {
        let _env = EnvGuard::set(&[("DATA_API_STRICT_SCOPE", Some("true"))]);
        assert_eq!(
            send(&router, "", None).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let _env = EnvGuard::set(&[("DATA_API_STRICT_SCOPE", None)]);
    assert_eq!(send(&router, "", None).await.status(), StatusCode::OK);
    cleanup(pool).await;
}

/// 全域滿載回 429 與 Retry-After，不消耗租約；名額空出後同一張租約可用。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn busy_does_not_consume_lease() {
    let Some(pool) = pool_or_skip("busy_does_not_consume_lease").await else {
        return;
    };
    let member = test_member(pool).await;
    let gate = Arc::new(AnalysisGate::new(1, 0));
    let router = fake_router(state(Arc::clone(&gate), Duration::from_secs(5)));
    let lease = issue(pool, member, "busy").await;

    let held = gate.acquire_internal().await.expect("佔住名額");
    let response = send(&router, "", Some(&lease)).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get(header::RETRY_AFTER),
        Some(&HeaderValue::from_static("10"))
    );
    assert_eq!(lease_times(pool, &lease.id).await, (false, false));
    drop(held);
    assert_eq!(
        send(&router, "", Some(&lease)).await.status(),
        StatusCode::OK
    );
    cleanup(pool).await;
}

/// 逾時回 504、finished_at 有值、名額已歸還。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn timeout_writes_finished_at_and_returns_slot() {
    let Some(pool) = pool_or_skip("timeout_writes_finished_at_and_returns_slot").await else {
        return;
    };
    let member = test_member(pool).await;
    let gate = Arc::new(AnalysisGate::new(1, 0));
    let router = fake_router(state(Arc::clone(&gate), Duration::from_millis(100)));
    let lease = issue(pool, member, "timeout").await;
    assert_eq!(
        send(&router, "?sleep_ms=2000", Some(&lease)).await.status(),
        StatusCode::GATEWAY_TIMEOUT
    );
    assert_eq!(lease_times(pool, &lease.id).await, (true, true));
    assert_eq!(gate.available(), 1);
    cleanup(pool).await;
}

/// 用戶端提前斷線：名額在 task 結束後才歸還，finished_at 照樣寫入。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn client_disconnect_keeps_slot_until_task_ends() {
    let Some(pool) = pool_or_skip("client_disconnect_keeps_slot_until_task_ends").await else {
        return;
    };
    let member = test_member(pool).await;
    let gate = Arc::new(AnalysisGate::new(1, 0));
    let router = fake_router(state(Arc::clone(&gate), Duration::from_secs(5)));
    let lease = issue(pool, member, "disconnect").await;

    // 用戶端 200 毫秒後放棄：外層 future 被丟棄
    let abandoned = tokio::time::timeout(
        Duration::from_millis(200),
        send(&router, "?sleep_ms=1000", Some(&lease)),
    )
    .await;
    assert!(abandoned.is_err());
    assert_eq!(gate.available(), 0, "task 還在跑，名額不能先歸還");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(gate.available(), 1);
    assert_eq!(lease_times(pool, &lease.id).await, (true, true));
    cleanup(pool).await;
}

/// 逾時後資料庫端的慢查詢也會被 statement_timeout 中止，不留下執行中的查詢。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn statement_timeout_stops_query_after_request_timeout() {
    let Some(pool) = pool_or_skip("statement_timeout_stops_query_after_request_timeout").await
    else {
        return;
    };
    let router = fake_router(state(
        Arc::new(AnalysisGate::new(1, 0)),
        Duration::from_millis(500),
    ));
    assert_eq!(
        send(&router, "?slow_sql=true", None).await.status(),
        StatusCode::GATEWAY_TIMEOUT
    );
    // 一般查詢預算的 statement_timeout 是 6 秒
    tokio::time::sleep(QueryBudget::General.statement_timeout() + Duration::from_secs(1)).await;
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity
         WHERE state = 'active' AND query LIKE '%lease-statement-timeout-probe%'
           AND query NOT LIKE '%pg_stat_activity%'",
    )
    .fetch_one(pool)
    .await
    .expect("查詢 pg_stat_activity");
    assert_eq!(running, 0);
}
