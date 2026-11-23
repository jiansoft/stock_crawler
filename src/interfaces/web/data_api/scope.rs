//! 持股類端點（`/api/v1/portfolio/*`）的金鑰與資料範圍檢查 middleware（會員系統計畫 5.0.1）。
//!
//! - 金鑰分兩把：市場類端點用 `DATA_API_KEY`；持股類端點只接受 `DATA_API_PERSONAL_KEY`。
//! - 資料範圍：`household_id` 必填、`member_id` 選填（0 或不帶為該家庭合計）；指定的會員必須屬於
//!   該家庭（未刪除），不符回 403。Rust 不做權限判斷，只驗證成員與家庭的關係。
//! - 相容開關 `DATA_API_STRICT_SCOPE`（預設 false）：關閉時兩把金鑰都收、缺 `household_id`
//!   視為家庭 #1，行為與舊版相同，但每次都寫一行警告日誌；帶了 `household_id` 時照新規則驗證。
//!
//! `household_id`／`member_id` 從 query string 讀取；PUT／POST 另從 JSON body 讀取（body 會
//! 先讀入記憶體，再原封不動交給 handler）。

use axum::{
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use super::handlers::error_response;
use crate::domain::portfolio::view::{DEFAULT_HOUSEHOLD_ID, HoldingView};
use crate::infra::database;

/// 持股類端點 request body 的上限（目標配置最多數十項，1 MB 綽綽有餘）。
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// 相容開關的環境變數名稱。
pub(crate) const STRICT_SCOPE_ENV: &str = "DATA_API_STRICT_SCOPE";
/// 持股類端點專用金鑰的環境變數名稱。
pub(crate) const PERSONAL_KEY_ENV: &str = "DATA_API_PERSONAL_KEY";
/// 市場類端點金鑰的環境變數名稱。
pub(crate) const MARKET_KEY_ENV: &str = "DATA_API_KEY";
/// 註冊模式（stock_go 的設定，Rust 只用來做啟動檢查）。
pub(crate) const REGISTRATION_MODE_ENV: &str = "REGISTRATION_MODE";

/// 呼叫端帶的是哪一把金鑰。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuppliedKey {
    /// `DATA_API_PERSONAL_KEY`。
    Personal,
    /// `DATA_API_KEY`（市場類）。
    Market,
    /// 沒帶、格式錯誤或兩把都不符。
    Invalid,
}

/// 金鑰檢查的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyDecision {
    /// 放行。
    Allow,
    /// 放行，但要寫相容日誌（開關關閉時以市場金鑰呼叫持股端點）。
    AllowLegacy,
    /// 拒絕（401）。
    Reject,
}

/// 依金鑰與開關決定是否放行持股類端點。
pub(crate) fn key_decision(key: SuppliedKey, strict: bool) -> KeyDecision {
    match (key, strict) {
        (SuppliedKey::Personal, _) => KeyDecision::Allow,
        (SuppliedKey::Market, false) => KeyDecision::AllowLegacy,
        (SuppliedKey::Market, true) | (SuppliedKey::Invalid, _) => KeyDecision::Reject,
    }
}

/// 資料範圍檢查的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeDecision {
    /// 不需查資料庫即可放行（沒指定會員，或開關關閉且沒帶家庭）。
    Allow,
    /// 放行，但要寫相容日誌（開關關閉時缺 household_id，視為家庭 #1）。
    AllowLegacy,
    /// 指定了會員：要查資料庫確認他屬於這個家庭。
    VerifyMember { household_id: i64, member_id: i64 },
    /// 缺 household_id（400）。
    MissingHousehold,
    /// household_id 或 member_id 格式不合法（422）。
    Invalid(&'static str),
}

/// 依參數與開關決定資料範圍要怎麼檢查。
pub(crate) fn scope_decision(
    household_id: Option<i64>,
    member_id: Option<i64>,
    strict: bool,
) -> ScopeDecision {
    let member_id = member_id.unwrap_or(0);
    if member_id < 0 {
        return ScopeDecision::Invalid("member_id 不可為負數");
    }
    match household_id {
        None if strict => ScopeDecision::MissingHousehold,
        // 舊版呼叫端：行為與舊版相同（任何 member_id 都照舊查詢），只寫日誌
        None => ScopeDecision::AllowLegacy,
        Some(id) if id <= 0 => ScopeDecision::Invalid("household_id 必須為正整數"),
        Some(_) if member_id == 0 => ScopeDecision::Allow,
        Some(household_id) => ScopeDecision::VerifyMember {
            household_id,
            member_id,
        },
    }
}

/// 開關是否開啟：`true`／`1`（不分大小寫）為開啟，其餘（含未設定）為關閉。
pub(crate) fn strict_scope_enabled() -> bool {
    std::env::var(STRICT_SCOPE_ENV)
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1"))
}

/// 啟動檢查：開放註冊（`REGISTRATION_MODE` 不是 `closed`）時，相容開關必須開啟，
/// 否則持股端點仍接受市場金鑰與缺家庭的呼叫，外部會員的資料可能被讀到。
///
/// # Errors
///
/// 註冊模式不是 closed 而相容開關關閉時回錯誤，呼叫端應拒絕啟動。
pub(crate) fn validate_startup(
    registration_mode: Option<&str>,
    strict: bool,
) -> anyhow::Result<()> {
    let mode = registration_mode
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
        .unwrap_or("closed");
    if !mode.eq_ignore_ascii_case("closed") && !strict {
        anyhow::bail!(
            "{REGISTRATION_MODE_ENV}={mode} 時必須設定 {STRICT_SCOPE_ENV}=true（持股類端點不可再接受舊呼叫方式）"
        );
    }
    Ok(())
}

/// 以固定時間比較判斷呼叫端帶的是哪一把金鑰；環境變數未設定的那一把視為不存在。
fn classify_key(supplied: Option<&str>) -> SuppliedKey {
    let Some(supplied) = supplied else {
        return SuppliedKey::Invalid;
    };
    let matches = |name: &str| {
        std::env::var(name).is_ok_and(|expected| {
            !expected.is_empty() && bool::from(expected.as_bytes().ct_eq(supplied.as_bytes()))
        })
    };
    if matches(PERSONAL_KEY_ENV) {
        SuppliedKey::Personal
    } else if matches(MARKET_KEY_ENV) {
        SuppliedKey::Market
    } else {
        SuppliedKey::Invalid
    }
}

/// 從 query string 或 JSON body 取出的範圍參數；其他欄位忽略。
#[derive(Debug, Default, Deserialize)]
struct ScopeParams {
    household_id: Option<i64>,
    member_id: Option<i64>,
}

/// 持股類端點的 middleware：金鑰、household_id 與成員歸屬。
pub(super) async fn require_personal_scope(request: Request<Body>, next: Next) -> Response {
    let strict = strict_scope_enabled();
    let path = request.uri().path().to_owned();
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match key_decision(classify_key(supplied), strict) {
        KeyDecision::Reject => return error_response(StatusCode::UNAUTHORIZED, "未授權"),
        KeyDecision::AllowLegacy => {
            tracing::warn!(path = %path, "Data API 相容模式：以 DATA_API_KEY 呼叫持股類端點");
        }
        KeyDecision::Allow => {}
    }

    // 範圍參數：query 優先，PUT／POST 再看 JSON body
    let query: ScopeParams = axum::extract::Query::try_from_uri(request.uri())
        .map(|axum::extract::Query(params)| params)
        .unwrap_or_default();
    let (request, body_params) = if matches!(*request.method(), Method::PUT | Method::POST) {
        let (parts, body) = request.into_parts();
        let bytes: Bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
            Ok(bytes) => bytes,
            Err(_) => return error_response(StatusCode::PAYLOAD_TOO_LARGE, "request body 過大"),
        };
        let params: ScopeParams = serde_json::from_slice(&bytes).unwrap_or_default();
        (Request::from_parts(parts, Body::from(bytes)), params)
    } else {
        (request, ScopeParams::default())
    };
    let household_id = query.household_id.or(body_params.household_id);
    let member_id = query.member_id.or(body_params.member_id);

    match scope_decision(household_id, member_id, strict) {
        ScopeDecision::Allow => {}
        ScopeDecision::AllowLegacy => {
            tracing::warn!(path = %path, "Data API 相容模式：持股類端點缺少 household_id，視為家庭 #1");
        }
        ScopeDecision::MissingHousehold => {
            return error_response(StatusCode::BAD_REQUEST, "缺少 household_id");
        }
        ScopeDecision::Invalid(message) => {
            return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
        }
        ScopeDecision::VerifyMember {
            household_id,
            member_id,
        } => match member_in_household(household_id, member_id).await {
            Ok(true) => {}
            Ok(false) => {
                return error_response(StatusCode::FORBIDDEN, "member_id 不屬於這個 household_id");
            }
            Err(why) => {
                tracing::error!("Data API 驗證家庭成員失敗: {why:#}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "伺服器內部錯誤");
            }
        },
    }
    // 驗證過的範圍交給 handler：持股查詢一律以 HoldingView 篩選，`member_id = 0` 只到這個家庭為止（7.6 N2）。
    let mut request = request;
    request
        .extensions_mut()
        .insert(holding_view(household_id, member_id));
    next.run(request).await
}

/// 依已驗證的參數組出持股範圍；相容模式缺 `household_id` 時為預設家庭。
pub(crate) fn holding_view(household_id: Option<i64>, member_id: Option<i64>) -> HoldingView {
    HoldingView::member(
        household_id.unwrap_or(DEFAULT_HOUSEHOLD_ID),
        member_id.unwrap_or(0),
    )
}

/// 會員是否屬於指定家庭且未刪除。
async fn member_in_household(household_id: i64, member_id: i64) -> anyhow::Result<bool> {
    let found: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM "Member"
               WHERE "Id" = $1 AND household_id = $2 AND deleted_at IS NULL
           )"#,
    )
    .bind(member_id)
    .bind(household_id)
    .fetch_one(database::get_connection())
    .await?;
    Ok(found)
}

#[cfg(test)]
mod tests;
