//! Data API Bearer token 驗證 middleware。
//!
//! 健康檢查與 Swagger UI 不會套用此 middleware；其餘 `/api/v1` 路徑都必須
//! 使用 `DATA_API_KEY`。比較採固定時間演算法，避免以提早結束的字串比較洩漏 key 前綴。

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;

use super::handlers::error_response;

/// 驗證 request 的 `Authorization: Bearer <DATA_API_KEY>` 標頭。
pub(super) async fn require_bearer_key(request: Request<Body>, next: Next) -> Response {
    // 未設定 key 時拒絕所有受保護請求，避免部署漏設環境變數而意外公開資料。
    let Ok(expected) = std::env::var("DATA_API_KEY") else {
        tracing::error!("DATA_API_KEY is not configured");
        return error_response(StatusCode::UNAUTHORIZED, "未授權");
    };
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let authorized =
        supplied.is_some_and(|value| expected.as_bytes().ct_eq(value.as_bytes()).into());
    if !authorized {
        return error_response(StatusCode::UNAUTHORIZED, "未授權");
    }
    next.run(request).await
}
