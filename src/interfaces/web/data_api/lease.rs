//! 重運算租約的 Rust 端（會員系統計畫 2.3「重運算租約規格」）。
//!
//! 套用在路徑含 `/backtest` 的 Data API 端點。處理順序：
//! 1. 讀取租約標頭（`X-Analysis-Lease-Id`、`X-Analysis-Lease-Secret`、`X-Analysis-Member-Id`）。
//!    沒帶：`DATA_API_STRICT_SCOPE` 開啟時回 403 `lease_invalid`；關閉時照舊放行並寫日誌。
//! 2. 取得全域並行名額（[`super::analysis_gate`]）；滿載回 429 `analysis_busy` 與
//!    `Retry-After: 10`，不消耗租約。
//! 3. 以單句 UPDATE 原子消耗租約；影響 0 列回 403 `lease_invalid`（不區分不存在、密鑰錯、
//!    已用過或過期）。
//! 4. 在獨立 task 內執行回測（重新套用查詢預算與請求上限），名額與收尾都由這個 task 持有：
//!    用戶端提前斷線時 task 照常跑完才歸還名額；成功、失敗與逾時都寫 `finished_at`。
//!    參數驗證失敗（400／422）不算消耗：把 `started_at` 改回 NULL，期限內可再用同一張租約。
//!
//! 租約密鑰只放標頭、不寫日誌；資料庫只存它的 SHA-256。

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};

use super::analysis_gate::{AnalysisGate, GATE};
use super::budget;
use super::handlers::error_response;
use super::scope::strict_scope_enabled;
use crate::core::shutdown::BACKGROUND_OPERATIONS;
use crate::infra::database::{self, QueryBudget};

/// 租約編號標頭。
pub(crate) const LEASE_ID_HEADER: &str = "x-analysis-lease-id";
/// 租約密鑰標頭（base64url 32 位元組）。
pub(crate) const LEASE_SECRET_HEADER: &str = "x-analysis-lease-secret";
/// 租約所屬會員標頭。
pub(crate) const LEASE_MEMBER_HEADER: &str = "x-analysis-member-id";
/// 全域滿載時建議的重試秒數。
const BUSY_RETRY_AFTER_SECS: &str = "10";

/// 中介層的設定：名額、請求上限與查詢預算。正式環境用 [`LeaseState::global`]，測試可注入。
#[derive(Debug, Clone)]
pub(crate) struct LeaseState {
    pub(crate) gate: Arc<AnalysisGate>,
    pub(crate) timeout: Duration,
    pub(crate) budget: QueryBudget,
}

impl LeaseState {
    /// 正式設定：全域號誌、回測請求上限 [`budget::HEAVY_TIMEOUT`]、重型查詢預算。
    pub(crate) fn global() -> Self {
        Self {
            gate: Arc::clone(&GATE),
            timeout: budget::HEAVY_TIMEOUT,
            budget: QueryBudget::Heavy,
        }
    }
}

/// 解析過的租約標頭。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaseHeaders {
    pub(crate) id: String,
    pub(crate) secret: String,
    pub(crate) member_id: i64,
}

/// 請求帶的租約。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LeaseInput {
    /// 三個標頭都沒帶。
    Absent,
    /// 三個標頭齊全且格式正確。
    Present(LeaseHeaders),
    /// 只帶了一部分，或格式錯誤。
    Malformed,
}

/// UUID 文字格式（8-4-4-4-12 個十六進位字元）。
fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.char_indices().all(|(index, ch)| match index {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}

/// base64url 編碼的 32 位元組（43 個字元，可帶一個 `=`）。
fn is_lease_secret(value: &str) -> bool {
    let trimmed = value.strip_suffix('=').unwrap_or(value);
    trimmed.len() == 43
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// 從標頭解析租約。
pub(crate) fn parse_lease(headers: &HeaderMap) -> LeaseInput {
    let get = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    match (
        get(LEASE_ID_HEADER),
        get(LEASE_SECRET_HEADER),
        get(LEASE_MEMBER_HEADER),
    ) {
        (None, None, None) => LeaseInput::Absent,
        (Some(id), Some(secret), Some(member)) => {
            let member_id = member.trim().parse::<i64>().ok().filter(|id| *id > 0);
            match member_id {
                Some(member_id) if is_uuid(id) && is_lease_secret(secret) => {
                    LeaseInput::Present(LeaseHeaders {
                        id: id.to_ascii_lowercase(),
                        secret: secret.strip_suffix('=').unwrap_or(secret).to_owned(),
                        member_id,
                    })
                }
                _ => LeaseInput::Malformed,
            }
        }
        _ => LeaseInput::Malformed,
    }
}

/// 403 `lease_invalid`。
fn lease_invalid() -> Response {
    error_response(StatusCode::FORBIDDEN, "lease_invalid")
}

/// 429 `analysis_busy` 與 `Retry-After`。
fn analysis_busy() -> Response {
    let mut response = error_response(StatusCode::TOO_MANY_REQUESTS, "analysis_busy");
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_static(BUSY_RETRY_AFTER_SECS),
    );
    response
}

/// 原子消耗租約：只有編號、會員、密鑰都對、尚未開始且未過期的那一列會被更新。
async fn consume(lease: &LeaseHeaders) -> anyhow::Result<bool> {
    let result = sqlx::query(
        r#"UPDATE member_analysis_lease SET started_at = now()
           WHERE id = $1::uuid AND member_id = $2
             AND secret_hash = sha256(decode(translate($3, '-_', '+/') || '=', 'base64'))
             AND started_at IS NULL AND expires_at > now()"#,
    )
    .bind(&lease.id)
    .bind(lease.member_id)
    .bind(&lease.secret)
    .execute(database::get_connection())
    .await?;
    Ok(result.rows_affected() == 1)
}

/// 收尾：成功、失敗與逾時都寫 `finished_at`。
async fn finish(lease_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE member_analysis_lease SET finished_at = now() WHERE id = $1::uuid AND finished_at IS NULL",
    )
    .bind(lease_id)
    .execute(database::get_connection())
    .await?;
    Ok(())
}

/// 參數驗證失敗不算消耗：把 `started_at` 改回 NULL，期限內可再用同一張租約。
async fn release(lease_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE member_analysis_lease SET started_at = NULL WHERE id = $1::uuid AND finished_at IS NULL",
    )
    .bind(lease_id)
    .execute(database::get_connection())
    .await?;
    Ok(())
}

/// 回應是不是參數驗證失敗（400／422），這種情況不消耗租約。
fn is_validation_failure(status: StatusCode) -> bool {
    status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY
}

/// 重運算端點的租約 middleware。
pub(crate) async fn require_lease(
    State(state): State<LeaseState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let lease = match parse_lease(request.headers()) {
        LeaseInput::Present(lease) => Some(lease),
        LeaseInput::Malformed => return lease_invalid(),
        LeaseInput::Absent if strict_scope_enabled() => return lease_invalid(),
        LeaseInput::Absent => {
            tracing::warn!(path = %path, "Data API 相容模式：回測請求未帶重運算租約");
            None
        }
    };

    let Some(permit) = state.gate.acquire_external(state.timeout).await else {
        return analysis_busy();
    };

    if let Some(lease) = &lease {
        match consume(lease).await {
            Ok(true) => {}
            Ok(false) => return lease_invalid(),
            Err(why) => {
                tracing::error!(path = %path, "消耗重運算租約失敗: {why:#}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "伺服器內部錯誤");
            }
        }
    }

    // 名額、背景工作 guard 與收尾都交給獨立 task：用戶端斷線時外層 future 被丟棄，
    // task 仍會跑完（最長為請求上限）才歸還名額並寫 finished_at。
    let guard = BACKGROUND_OPERATIONS.begin();
    let lease_id = lease.map(|lease| lease.id);
    let task = tokio::spawn(async move {
        let _guard = guard;
        let _permit = permit;
        let response = budget::enforce_with(state.budget, state.timeout, request, next).await;
        if let Some(lease_id) = lease_id {
            let result = if is_validation_failure(response.status()) {
                release(&lease_id).await
            } else {
                finish(&lease_id).await
            };
            if let Err(why) = result {
                tracing::error!("重運算租約收尾失敗: {why:#}");
            }
        }
        response
    });
    match task.await {
        Ok(response) => response,
        Err(why) => {
            tracing::error!(path = %path, "重運算 task 異常結束: {why}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "伺服器內部錯誤")
        }
    }
}

#[cfg(test)]
mod tests;
