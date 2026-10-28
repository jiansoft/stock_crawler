//! `/portfolio/targets` handlers：投資組合目標配置比例的查詢與設定。
//!
//! 僅為個人偏好的試算基準，不會自動交易。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{error_response, repository_error};
use crate::domain::portfolio::target::{PortfolioTargetRepository, TargetDimension};
use crate::infra::database::repository::portfolio_target::PgPortfolioTargetRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioTargetItemDto, PortfolioTargetsQueryParams, PortfolioTargetsResponse,
    PutPortfolioTargetsRequest,
};

/// 查詢目前已設定之投資組合目標配置比例。
#[utoipa::path(
    get,
    path = "/api/v1/portfolio/targets",
    tag = "data-api",
    params(PortfolioTargetsQueryParams),
    responses(
        (status = 200, body = PortfolioTargetsResponse),
        (status = 401, body = ErrorBody),
        (status = 422, body = ErrorBody),
        (status = 500, body = ErrorBody)
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn get_portfolio_targets(
    Query(params): Query<PortfolioTargetsQueryParams>,
) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }

    let dim_opt = match params.dimension.as_deref() {
        Some(s) if !s.is_empty() => match TargetDimension::parse(s) {
            Some(d) => Some(d),
            None => {
                return error_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "dimension 必須為 stock, industry, 或 etf_look_through",
                );
            }
        },
        _ => None,
    };

    let repo = PgPortfolioTargetRepository::new();
    let records = match repo.list_targets(&member_id.to_string(), dim_opt).await {
        Ok(r) => r,
        Err(e) => return repository_error(e),
    };

    let dimensions = match repo.configured_dimensions(&member_id.to_string()).await {
        Ok(value) => value,
        Err(error) => return repository_error(error),
    };
    let mut cash_percent_by_dimension: std::collections::BTreeMap<_, _> = dimensions
        .into_iter()
        .filter(|d| dim_opt.is_none_or(|selected| selected == *d))
        .map(|d| (d.as_str().to_owned(), 100.0))
        .collect();
    for record in &records {
        *cash_percent_by_dimension
            .entry(record.dimension.as_str().to_owned())
            .or_insert(100.0) -= record.target_percent;
    }
    for cash in cash_percent_by_dimension.values_mut() {
        *cash = (*cash * 100.0).round() / 100.0;
    }
    let target_cash_percent = dim_opt.map(|dim| {
        cash_percent_by_dimension
            .get(dim.as_str())
            .copied()
            .unwrap_or(100.0)
    });

    let response = PortfolioTargetsResponse {
        member_id,
        dimension: dim_opt
            .map(|d| d.as_str().to_string())
            .unwrap_or_else(|| "all".to_string()),
        targets: records
            .into_iter()
            .map(|r| PortfolioTargetItemDto {
                dimension: Some(r.dimension.as_str().to_owned()),
                target_key: r.key,
                target_percent: r.target_percent,
            })
            .collect(),
        target_cash_percent,
        cash_percent_by_dimension,
        disclaimer: "目標配置比例僅為個人偏好試算基準，非自動交易委託。",
    };

    Json(response).into_response()
}

/// 設定或更新投資組合目標配置比例。
#[utoipa::path(
    put,
    path = "/api/v1/portfolio/targets",
    tag = "data-api",
    request_body = PutPortfolioTargetsRequest,
    responses(
        (status = 200, body = PortfolioTargetsResponse),
        (status = 401, body = ErrorBody),
        (status = 422, body = ErrorBody),
        (status = 500, body = ErrorBody)
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn put_portfolio_targets(
    Json(body): Json<PutPortfolioTargetsRequest>,
) -> Response {
    let member_id = body.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }

    let dimension = match TargetDimension::parse(&body.dimension) {
        Some(d) => d,
        None => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "dimension 必須為 stock, industry, 或 etf_look_through",
            );
        }
    };

    let targets: Vec<(String, f64)> = body
        .targets
        .iter()
        .map(|t| (t.target_key.clone(), t.target_percent))
        .collect();

    if let Err(error) = crate::domain::portfolio::target::validate_targets(&targets) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string());
    }
    let repo = PgPortfolioTargetRepository::new();
    if let Err(e) = repo
        .save_targets(&member_id.to_string(), dimension, &targets)
        .await
    {
        return repository_error(e);
    }
    saved_targets_response(&repo, member_id, dimension).await
}

/// 儲存（或還原）後回傳該維度目前的目標配置與現金比例，PUT 與還原共用。
pub(super) async fn saved_targets_response(
    repo: &PgPortfolioTargetRepository,
    member_id: i64,
    dimension: TargetDimension,
) -> Response {
    let saved = match repo
        .list_targets(&member_id.to_string(), Some(dimension))
        .await
    {
        Ok(r) => r,
        Err(e) => return repository_error(e),
    };

    let saved_sum: f64 = saved.iter().map(|r| r.target_percent).sum();
    let response = PortfolioTargetsResponse {
        member_id,
        dimension: dimension.as_str().to_string(),
        targets: saved
            .into_iter()
            .map(|r| PortfolioTargetItemDto {
                dimension: Some(r.dimension.as_str().to_owned()),
                target_key: r.key,
                target_percent: r.target_percent,
            })
            .collect(),
        target_cash_percent: Some(((100.0 - saved_sum).max(0.0) * 100.0).round() / 100.0),
        cash_percent_by_dimension: std::collections::BTreeMap::from([(
            dimension.as_str().to_owned(),
            ((100.0 - saved_sum).max(0.0) * 100.0).round() / 100.0,
        )]),
        disclaimer: "目標配置比例僅為個人偏好試算基準，非自動交易委託。",
    };

    Json(response).into_response()
}
