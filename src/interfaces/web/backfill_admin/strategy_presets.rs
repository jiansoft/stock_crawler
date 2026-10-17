//! 具名策略的管理 API：列出、新增或覆蓋、刪除。
//!
//! 策略是條件選股回測的 query string（見 `strategy_preset` 表），Data API 的回測端點以
//! `preset=名稱` 引用；`alert` 為 true 的策略由 22:10 的策略換股通知執行。存之前用 Data API 的驗證
//! 檢查條件（[`validate_strategy_query`]），不正確回 422。

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::domain::strategy::{StrategyPreset, StrategyPresetRepository, normalize_name};
use crate::infra::database::repository::strategy_preset::PgStrategyPresetRepository;
use crate::interfaces::web::data_api::validate_strategy_query;

use super::dto::ErrorResponse;

/// 新增或覆蓋策略的 request body。
#[derive(Debug, Deserialize)]
pub(super) struct StrategyPresetRequest {
    /// 條件選股回測的 query string（例如 `min_rs_score=80&limit=20&from=2019-01-02`）。
    pub(super) query: String,
    /// 說明；省略為空字串。
    #[serde(default)]
    pub(super) description: String,
    /// 是否發換股通知；省略為 false。
    #[serde(default)]
    pub(super) alert: bool,
}

/// 一個策略的輸出。
#[derive(Debug, Serialize)]
pub(super) struct StrategyPresetBody {
    pub(super) name: String,
    pub(super) query: String,
    pub(super) description: String,
    pub(super) alert: bool,
}

impl From<StrategyPreset> for StrategyPresetBody {
    fn from(preset: StrategyPreset) -> Self {
        Self {
            name: preset.name,
            query: preset.query,
            description: preset.description,
            alert: preset.alert,
        }
    }
}

/// 具名策略的路由：`GET /api/strategy-presets`、`PUT`／`DELETE /api/strategy-presets/{name}`。
pub(super) fn router() -> Router {
    Router::new()
        .route("/api/strategy-presets", get(list_presets))
        .route(
            "/api/strategy-presets/{name}",
            axum::routing::put(save_preset).delete(delete_preset),
        )
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

fn internal(why: anyhow::Error) -> Response {
    tracing::error!(?why, "具名策略存取失敗");
    error(StatusCode::INTERNAL_SERVER_ERROR, "具名策略存取失敗")
}

/// 列出全部策略。
async fn list_presets() -> Response {
    match PgStrategyPresetRepository::new().list().await {
        Ok(presets) => Json(
            presets
                .into_iter()
                .map(StrategyPresetBody::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(why) => internal(why),
    }
}

/// 新增或覆蓋策略：名稱與條件檢查不過回 422。
async fn save_preset(Path(name): Path<String>, Json(req): Json<StrategyPresetRequest>) -> Response {
    let name = match normalize_name(&name) {
        Ok(name) => name,
        Err(message) => return error(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let query = req.query.trim().trim_start_matches('?').to_owned();
    if let Err(message) = validate_strategy_query(&query, req.alert).await {
        return error(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    let preset = StrategyPreset {
        name,
        query,
        description: req.description.trim().to_owned(),
        alert: req.alert,
    };
    match PgStrategyPresetRepository::new().save(&preset).await {
        Ok(()) => Json(StrategyPresetBody::from(preset)).into_response(),
        Err(why) => internal(why),
    }
}

/// 刪除策略；不存在回 404。
async fn delete_preset(Path(name): Path<String>) -> Response {
    match PgStrategyPresetRepository::new().delete(name.trim()).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(
            StatusCode::NOT_FOUND,
            format!("查無策略「{}」", name.trim()),
        ),
        Err(why) => internal(why),
    }
}

#[cfg(test)]
mod tests {
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
}
