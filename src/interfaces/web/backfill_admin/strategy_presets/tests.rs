//! 具名策略的管理 API：列出、新增或覆蓋、刪除的測試。

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use super::*;

/// 送出 PUT 並取回（狀態碼, 錯誤訊息）。
async fn put(name: &str, body: &str) -> (StatusCode, String) {
    let response = router()
        .oneshot(
            Request::put(format!("/api/strategy-presets/{name}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (
        status,
        json["error"].as_str().unwrap_or_default().to_owned(),
    )
}

/// 名稱不合規則、條件引用 preset、通知策略沒有 from、query 格式錯誤：都在碰資料庫前回 422。
#[tokio::test]
async fn invalid_presets_are_rejected_before_saving() {
    let (status, message) = put("a%26b", r#"{"query":"from=2019-01-02"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(message, "策略名稱不可包含 & = # ? 或控制字元");

    let (status, message) = put("ok", r#"{"query":"preset=x&limit=5"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(message, "策略條件不可再引用 preset");

    let (status, message) = put("ok", r#"{"query":"limit=5","alert":true}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(message, "發換股通知的策略必須指定 from");

    let (status, _) = put("ok", r#"{"query":"limit=abc"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
