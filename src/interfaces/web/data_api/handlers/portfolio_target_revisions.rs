//! `/portfolio/targets/revisions` handlers：目標配置的歷史版本與還原。
//!
//! 每次儲存目標配置前，該維度原本的整組會記成一版（最多保留 20 版）；還原就是用那一版的內容再儲存
//! 一次，所以還原前的設定也會留成一版。僅為個人偏好的試算基準，不會自動交易。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::portfolio_targets::saved_targets_response;
use super::{error_response, repository_error};
use crate::domain::portfolio::target::{PortfolioTargetRepository, TargetDimension};
use crate::infra::database::repository::portfolio_target::PgPortfolioTargetRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioTargetItemDto, PortfolioTargetRevisionDto,
    PortfolioTargetRevisionsQueryParams, PortfolioTargetRevisionsResponse,
    PortfolioTargetsResponse, RestorePortfolioTargetRevisionRequest,
};

/// 列出某會員某維度目標配置的歷史版本，由新到舊。
///
/// # Errors
///
/// `member_id` 為負數或 `dimension` 不合法回 422；驗證失敗回 401；資料庫查詢失敗回 500。
#[utoipa::path(
    get,
    path = "/api/v1/portfolio/targets/revisions",
    tag = "data-api",
    params(PortfolioTargetRevisionsQueryParams),
    responses(
        (status = 200, body = PortfolioTargetRevisionsResponse),
        (status = 401, body = ErrorBody),
        (status = 422, body = ErrorBody),
        (status = 500, body = ErrorBody)
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn list_portfolio_target_revisions(
    Query(params): Query<PortfolioTargetRevisionsQueryParams>,
) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let Some(dimension) = TargetDimension::parse(&params.dimension) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "dimension 必須為 stock, industry, 或 etf_look_through",
        );
    };

    let revisions = match PgPortfolioTargetRepository::new()
        .list_revisions(&member_id.to_string(), dimension)
        .await
    {
        Ok(revisions) => revisions,
        Err(error) => return repository_error(error),
    };

    Json(PortfolioTargetRevisionsResponse {
        member_id,
        dimension: dimension.as_str().to_string(),
        revisions: revisions
            .into_iter()
            .map(|revision| {
                let total: f64 = revision.targets.iter().map(|(_, percent)| percent).sum();
                PortfolioTargetRevisionDto {
                    id: revision.id,
                    saved_time: revision.saved_time.to_rfc3339(),
                    target_cash_percent: ((100.0 - total).max(0.0) * 100.0).round() / 100.0,
                    targets: revision
                        .targets
                        .into_iter()
                        .map(|(key, percent)| PortfolioTargetItemDto {
                            dimension: None,
                            target_key: key,
                            target_percent: percent,
                        })
                        .collect(),
                }
            })
            .collect(),
        disclaimer: "目標配置比例僅為個人偏好試算基準，非自動交易委託。",
    })
    .into_response()
}

/// 把目標配置還原成某個歷史版本；還原前的設定會記成新的一版，可以再還原回去。
///
/// # Errors
///
/// `member_id` 為負數回 422；版本不存在或不屬於這個會員回 404；驗證失敗回 401；資料庫失敗回 500。
#[utoipa::path(
    post,
    path = "/api/v1/portfolio/targets/revisions/{id}/restore",
    tag = "data-api",
    params(("id" = i64, Path, description = "版本編號（見 `/portfolio/targets/revisions`）")),
    request_body = RestorePortfolioTargetRevisionRequest,
    responses(
        (status = 200, body = PortfolioTargetsResponse),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 422, body = ErrorBody),
        (status = 500, body = ErrorBody)
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn restore_portfolio_target_revision(
    Path(id): Path<i64>,
    Json(body): Json<RestorePortfolioTargetRevisionRequest>,
) -> Response {
    let member_id = body.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let repo = PgPortfolioTargetRepository::new();
    let revision = match repo.get_revision(&member_id.to_string(), id).await {
        Ok(Some(revision)) => revision,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "找不到這個目標配置版本"),
        Err(error) => return repository_error(error),
    };
    if let Err(error) = repo
        .save_targets(
            &member_id.to_string(),
            revision.dimension,
            &revision.targets,
        )
        .await
    {
        return repository_error(error);
    }
    saved_targets_response(&repo, member_id, revision.dimension).await
}
