//! `/portfolio/rebalance` 與 `/portfolio/targets` handlers：持股再平衡試算與目標配置管理。
//!
//! 僅提供試算與調整建議提示，絕不下單交易。所有結果附帶免責聲明。

use std::collections::HashSet;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{database_error, error_response, repository_error};
use crate::domain::portfolio::rebalance::{
    CurrentPosition, EtfLookThroughComponent, RebalanceParams, calculate_rebalance,
};
use crate::domain::portfolio::target::{PortfolioTargetRepository, TargetDimension};
use crate::infra::database::{self, repository::portfolio_target::PgPortfolioTargetRepository};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, EtfLookThroughRebalanceItemDto, IndustryRebalanceItemDto,
    PortfolioRebalanceQueryParams, PortfolioRebalanceResponse, PortfolioTargetItemDto,
    PortfolioTargetsQueryParams, PortfolioTargetsResponse, PutPortfolioTargetsRequest,
    RebalanceCashSummaryDto, StockRebalanceItemDto,
};

/// 依代號合併之持股部位。
#[derive(sqlx::FromRow)]
struct HeldRow {
    stock_symbol: String,
    shares: i64,
    name: String,
    industry_name: String,
    closing_price: Option<f64>,
    is_etf: bool,
}

/// ETF 成分股查詢列。
#[derive(sqlx::FromRow)]
struct EtfConstituentRow {
    etf_symbol: String,
    holding_symbol: String,
    holding_name: String,
    weight: f64,
}

/// 查詢持股再平衡建議。
///
/// 依設定之目標配置與目前持有市值試算加減碼股數、金額與手續費等交易成本。
/// 僅供參考，不構成任何投資建議或交易委託。
#[utoipa::path(
    get,
    path = "/api/v1/portfolio/rebalance",
    tag = "data-api",
    params(PortfolioRebalanceQueryParams),
    responses(
        (status = 200, body = PortfolioRebalanceResponse),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 422, body = ErrorBody),
        (status = 500, body = ErrorBody)
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn portfolio_rebalance(
    Query(params): Query<PortfolioRebalanceQueryParams>,
) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }

    let dim_str = params.dimension.as_deref().unwrap_or("stock");
    let dimension = match TargetDimension::parse(dim_str) {
        Some(d) => d,
        None => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "dimension 必須為 stock, industry, 或 etf_look_through",
            );
        }
    };

    let lot_size = params.lot_size.unwrap_or(1000);
    if lot_size != 1 && lot_size != 1000 {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "lot_size 必須為 1 或 1000",
        );
    }

    let tolerance = params.tolerance_percent.unwrap_or(2.0);
    if tolerance < 0.0 {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "tolerance_percent 不可為負數",
        );
    }

    let cash = params.available_cash.unwrap_or(0.0);
    if cash < 0.0 {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "available_cash 不可為負數",
        );
    }

    let fee_discount = params.fee_discount.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&fee_discount) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "fee_discount 必須介於 0.0 至 1.0",
        );
    }

    // 1. 載入目標設定
    let target_repo = PgPortfolioTargetRepository::new();
    let target_records = match target_repo
        .list_targets(&member_id.to_string(), Some(dimension))
        .await
    {
        Ok(records) => records,
        Err(err) => return repository_error(err),
    };

    if target_records.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            "尚未設定此維度的目標配置比例，請先使用 PUT /api/v1/portfolio/targets 設定目標",
        );
    }

    let targets: Vec<(String, f64)> = target_records
        .iter()
        .map(|r| (r.key.clone(), r.target_percent))
        .collect();

    // 2. 查詢現有持股部位
    let held_sql = r#"
        WITH held AS (
            SELECT security_code AS stock_symbol,
                SUM(share_quantity)::bigint AS shares
            FROM stock_ownership_details
            WHERE NOT is_sold AND ($1::bigint = 0 OR member_id = $1)
            GROUP BY security_code
            HAVING SUM(share_quantity) > 0
        )
        SELECT held.stock_symbol::text AS stock_symbol,
            held.shares,
            COALESCE(e.name, s."Name", held.stock_symbol::text) AS name,
            COALESCE(si.name, '') AS industry_name,
            CASE WHEN q.closing_price > 0 THEN q.closing_price::float8 END AS closing_price,
            e.stock_symbol IS NOT NULL AS is_etf
        FROM held
        LEFT JOIN stocks s ON s.stock_symbol = held.stock_symbol
        LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
        LEFT JOIN last_daily_quotes q ON q.stock_symbol = held.stock_symbol
        LEFT JOIN etf_snapshot e ON e.stock_symbol = held.stock_symbol
        ORDER BY held.stock_symbol
    "#;

    let held_rows: Vec<HeldRow> = match sqlx::query_as(held_sql)
        .bind(member_id)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(err) => return database_error(err),
    };

    let mut held_symbols: HashSet<String> =
        held_rows.iter().map(|r| r.stock_symbol.clone()).collect();
    let mut positions: Vec<CurrentPosition> = held_rows
        .iter()
        .map(|r| CurrentPosition {
            symbol: r.stock_symbol.clone(),
            name: r.name.clone(),
            shares: r.shares,
            current_price: r.closing_price.unwrap_or(0.0),
            industry: r.industry_name.clone(),
        })
        .collect();

    // 3. 若目標設定中有未持有標的，查詢其最新報價與產業以供加碼試算
    let mut missing_symbols: Vec<String> = Vec::new();
    if dimension == TargetDimension::Stock || dimension == TargetDimension::EtfLookThrough {
        for (sym, _) in &targets {
            if !held_symbols.contains(sym) {
                missing_symbols.push(sym.clone());
                held_symbols.insert(sym.clone());
            }
        }
    }

    if !missing_symbols.is_empty() {
        let missing_sql = r#"
            SELECT s.stock_symbol::text AS stock_symbol,
                0::bigint AS shares,
                COALESCE(e.name, s."Name", s.stock_symbol::text) AS name,
                COALESCE(si.name, '') AS industry_name,
                CASE WHEN q.closing_price > 0 THEN q.closing_price::float8 END AS closing_price,
                e.stock_symbol IS NOT NULL AS is_etf
            FROM stocks s
            LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
            LEFT JOIN last_daily_quotes q ON q.stock_symbol = s.stock_symbol
            LEFT JOIN etf_snapshot e ON e.stock_symbol = s.stock_symbol
            WHERE s.stock_symbol = ANY($1)
        "#;

        let missing_rows: Vec<HeldRow> = match sqlx::query_as(missing_sql)
            .bind(&missing_symbols)
            .fetch_all(database::get_connection())
            .await
        {
            Ok(rows) => rows,
            Err(err) => return database_error(err),
        };

        for r in missing_rows {
            positions.push(CurrentPosition {
                symbol: r.stock_symbol,
                name: r.name,
                shares: 0,
                current_price: r.closing_price.unwrap_or(0.0),
                industry: r.industry_name,
            });
        }
    }

    // 4. 若為 ETF 穿透維度，取得持有 ETF 之成分股權重
    let mut etf_components: Vec<EtfLookThroughComponent> = Vec::new();
    if dimension == TargetDimension::EtfLookThrough {
        let etf_symbols: Vec<String> = held_rows
            .iter()
            .filter(|r| r.is_etf || r.stock_symbol.starts_with("00"))
            .map(|r| r.stock_symbol.clone())
            .collect();

        if !etf_symbols.is_empty() {
            let constituents_sql = r#"
                SELECT stock_symbol::text AS etf_symbol,
                       holding_symbol::text AS holding_symbol,
                       COALESCE(holding_name, holding_symbol::text) AS holding_name,
                       weight::float8 AS weight
                FROM etf_holding
                WHERE stock_symbol = ANY($1)
                ORDER BY stock_symbol, rank
            "#;

            let constituent_rows: Vec<EtfConstituentRow> = match sqlx::query_as(constituents_sql)
                .bind(&etf_symbols)
                .fetch_all(database::get_connection())
                .await
            {
                Ok(rows) => rows,
                Err(err) => return database_error(err),
            };

            etf_components = constituent_rows
                .into_iter()
                .map(|r| EtfLookThroughComponent {
                    stock_symbol: r.holding_symbol,
                    name: r.holding_name,
                    weight_percent: r.weight,
                    etf_symbol: r.etf_symbol,
                })
                .collect();
        }
    }

    // 5. 執行再平衡純運算
    let rebalance_params = RebalanceParams {
        lot_size,
        tolerance_percent: tolerance,
        available_cash: cash,
        fee_rate: 0.001425,
        tax_rate: 0.003,
        fee_discount,
    };

    let result = match calculate_rebalance(
        dimension,
        &targets,
        &positions,
        &etf_components,
        &rebalance_params,
    ) {
        Ok(res) => res,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, &err.to_string()),
    };

    // 6. 封裝 API 回應
    let response = PortfolioRebalanceResponse {
        member_id,
        dimension: dimension.as_str().to_string(),
        total_equity_value: result.total_equity_value,
        total_portfolio_value: result.total_portfolio_value,
        target_cash_percent: result.target_cash_percent,
        tolerance_percent: result.tolerance_percent,
        lot_size: result.lot_size,
        stock_items: result
            .stock_items
            .into_iter()
            .map(|item| StockRebalanceItemDto {
                symbol: item.symbol,
                name: item.name,
                current_shares: item.current_shares,
                current_price: item.current_price,
                current_value: item.current_value,
                current_weight: item.current_weight,
                target_weight: item.target_weight,
                deviation_percent: item.deviation_percent,
                action: item.action.to_string(),
                suggested_shares: item.suggested_shares,
                suggested_amount: item.suggested_amount,
                estimated_cost: item.estimated_cost,
                net_amount: item.net_amount,
            })
            .collect(),
        industry_items: result
            .industry_items
            .into_iter()
            .map(|item| IndustryRebalanceItemDto {
                industry: item.industry,
                current_value: item.current_value,
                current_weight: item.current_weight,
                target_weight: item.target_weight,
                deviation_percent: item.deviation_percent,
                candidate_stocks: item.candidate_stocks,
            })
            .collect(),
        etf_items: result
            .etf_items
            .into_iter()
            .map(|item| EtfLookThroughRebalanceItemDto {
                stock_symbol: item.stock_symbol,
                name: item.name,
                direct_value: item.direct_value,
                indirect_value: item.indirect_value,
                total_value: item.total_value,
                current_weight: item.current_weight,
                target_weight: item.target_weight,
                deviation_percent: item.deviation_percent,
            })
            .collect(),
        cash_summary: RebalanceCashSummaryDto {
            initial_cash: result.cash_summary.initial_cash,
            estimated_sell_net: result.cash_summary.estimated_sell_net,
            estimated_buy_total: result.cash_summary.estimated_buy_total,
            available_capital: result.cash_summary.available_capital,
            required_additional_cash: result.cash_summary.required_additional_cash,
            remaining_cash: result.cash_summary.remaining_cash,
        },
        disclaimer: result.disclaimer,
    };

    Json(response).into_response()
}

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

    let sum: f64 = records.iter().map(|r| r.target_percent).sum();
    let target_cash_percent = (100.0 - sum).max(0.0);

    let response = PortfolioTargetsResponse {
        member_id,
        dimension: dim_opt
            .map(|d| d.as_str().to_string())
            .unwrap_or_else(|| "all".to_string()),
        targets: records
            .into_iter()
            .map(|r| PortfolioTargetItemDto {
                target_key: r.key,
                target_percent: r.target_percent,
            })
            .collect(),
        target_cash_percent,
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

    let sum: f64 = body.targets.iter().map(|t| t.target_percent).sum();
    if sum > 100.0001 {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("同一維度的目標權重總和 ({sum:.2}%) 不得超過 100%"),
        );
    }
    for t in &body.targets {
        if t.target_percent < 0.0 || t.target_percent > 100.0 {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!(
                    "目標權重 ({}: {}%) 必須介於 0% 至 100%",
                    t.target_key, t.target_percent
                ),
            );
        }
    }

    let targets: Vec<(String, f64)> = body
        .targets
        .iter()
        .map(|t| (t.target_key.clone(), t.target_percent))
        .collect();

    let repo = PgPortfolioTargetRepository::new();
    if let Err(e) = repo
        .save_targets(&member_id.to_string(), dimension, &targets)
        .await
    {
        return repository_error(e);
    }

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
                target_key: r.key,
                target_percent: r.target_percent,
            })
            .collect(),
        target_cash_percent: (100.0 - saved_sum).max(0.0),
        disclaimer: "目標配置比例僅為個人偏好試算基準，非自動交易委託。",
    };

    Json(response).into_response()
}
