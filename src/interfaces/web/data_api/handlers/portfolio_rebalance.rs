//! `/portfolio/rebalance` handler：持股再平衡試算。
//!
//! 僅提供試算與調整建議提示，絕不下單交易。所有結果附帶免責聲明。持股、目標中尚未持有的標的與 ETF
//! 前十大成分由 [`PgPortfolioTargetRepository`]（[`PortfolioPositionQuery`]）讀取，與每週市場摘要的
//! 配置偏離提醒同一份來源；目標配置的查詢與設定見 [`super::portfolio_targets`]。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{error_response, repository_error};
use crate::domain::portfolio::costs::{FEE_RATE, STOCK_TAX_RATE};
use crate::domain::portfolio::rebalance::{
    CurrentPosition, DatedEtfComponent, EtfLookThroughComponent, PortfolioPositionQuery,
    PortfolioRebalanceResult, RebalanceParams, calculate_rebalance,
};
use crate::domain::portfolio::target::{PortfolioTargetRepository, TargetDimension};
use crate::infra::database::repository::portfolio_target::PgPortfolioTargetRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, EtfLookThroughRebalanceItemDto, IndustryRebalanceItemDto,
    PortfolioRebalanceQueryParams, PortfolioRebalanceResponse, RebalanceCashSummaryDto,
    RebalanceEtfCoverageDto, StockRebalanceItemDto,
};

/// 驗證過的查詢參數。
struct Validated {
    member_id: i64,
    dimension: TargetDimension,
    params: RebalanceParams,
}

/// 驗證查詢參數並套用預設值；不合法時回錯誤訊息（422）。
fn validate(params: &PortfolioRebalanceQueryParams) -> Result<Validated, &'static str> {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return Err("member_id 不可為負數");
    }
    let Some(dimension) = TargetDimension::parse(params.dimension.as_deref().unwrap_or("stock"))
    else {
        return Err("dimension 必須為 stock, industry, 或 etf_look_through");
    };
    let lot_size = params.lot_size.unwrap_or(1000);
    if lot_size != 1 && lot_size != 1000 {
        return Err("lot_size 必須為 1 或 1000");
    }
    let tolerance = params.tolerance_percent.unwrap_or(2.0);
    if !tolerance.is_finite() || !(0.0..=100.0).contains(&tolerance) {
        return Err("tolerance_percent 須為 0 至 100 的有限數值");
    }
    let cash = params.available_cash.unwrap_or(0.0);
    if !cash.is_finite() || cash < 0.0 {
        return Err("available_cash 須為非負有限數值");
    }
    let fee_discount = params.fee_discount.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&fee_discount) {
        return Err("fee_discount 必須介於 0.0 至 1.0");
    }
    Ok(Validated {
        member_id,
        dimension,
        params: RebalanceParams {
            lot_size,
            tolerance_percent: tolerance,
            available_cash: cash,
            fee_rate: FEE_RATE,
            tax_rate: STOCK_TAX_RATE,
            fee_discount,
        },
    })
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
    let Validated {
        member_id,
        dimension,
        params,
    } = match validate(&params) {
        Ok(validated) => validated,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    // 1. 載入目標設定
    let repo = PgPortfolioTargetRepository::new();
    let member = member_id.to_string();
    let target_records = match repo.list_targets(&member, Some(dimension)).await {
        Ok(records) => records,
        Err(error) => return repository_error(error),
    };
    let configured = match repo.has_configuration(&member, dimension).await {
        Ok(value) => value,
        Err(error) => return repository_error(error),
    };
    if target_records.is_empty() && !configured {
        return error_response(
            StatusCode::NOT_FOUND,
            "尚未設定此維度的目標配置比例，請先使用 PUT /api/v1/portfolio/targets 設定目標",
        );
    }
    let targets: Vec<(String, f64)> = target_records
        .iter()
        .map(|r| (r.key.clone(), r.target_percent))
        .collect();

    // 2. 現有持股；個股與 ETF 穿透維度另以零股數補上目標中尚未持有的標的，供加碼試算
    let extra_symbols: Vec<String> = match dimension {
        TargetDimension::Stock | TargetDimension::EtfLookThrough => {
            targets.iter().map(|(key, _)| key.clone()).collect()
        }
        TargetDimension::Industry => Vec::new(),
    };
    let positions = match repo.fetch_positions(member_id, &extra_symbols).await {
        Ok(positions) => positions,
        Err(error) => return repository_error(error),
    };

    // 3. 持有 ETF 的前十大成分（ETF 穿透試算與涵蓋率）
    let etf_symbols: Vec<String> = held_etfs(&positions)
        .map(|position| position.symbol.clone())
        .collect();
    let components = match repo.fetch_etf_components(&etf_symbols).await {
        Ok(components) => components,
        Err(error) => return repository_error(error),
    };

    // 缺價或不存在的目標不可默默當成零市值，否則會扭曲其他持股權重。
    let unpriced = positions
        .iter()
        .any(|p| !p.current_price.is_finite() || p.current_price <= 0.0);
    let unknown_target = dimension != TargetDimension::Industry
        && targets
            .iter()
            .any(|(key, _)| !positions.iter().any(|p| &p.symbol == key));
    if unpriced || unknown_target {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "持股或目標缺少有效收盤價／股票主檔，無法可靠試算",
        );
    }
    let coverage = etf_coverage(&positions, &components);

    // 4. 執行再平衡純運算
    let etf_components: Vec<EtfLookThroughComponent> = components
        .into_iter()
        .map(|dated| dated.component)
        .collect();
    match calculate_rebalance(dimension, &targets, &positions, &etf_components, &params) {
        Ok(result) => Json(response(member_id, dimension, coverage, result)).into_response(),
        Err(error) => error_response(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string()),
    }
}

/// 實際持有（股數大於 0）的 ETF，依持股順序。
fn held_etfs(positions: &[CurrentPosition]) -> impl Iterator<Item = &CurrentPosition> {
    positions
        .iter()
        .filter(|position| position.is_etf && position.shares > 0)
}

/// 每檔持有 ETF 的前十大成分涵蓋比例、未涵蓋市值與成分資料日期。
fn etf_coverage(
    positions: &[CurrentPosition],
    components: &[DatedEtfComponent],
) -> Vec<RebalanceEtfCoverageDto> {
    held_etfs(positions)
        .map(|position| {
            let own: Vec<&DatedEtfComponent> = components
                .iter()
                .filter(|dated| dated.component.etf_symbol == position.symbol)
                .collect();
            let covered: f64 = own.iter().map(|dated| dated.component.weight_percent).sum();
            RebalanceEtfCoverageDto {
                symbol: position.symbol.clone(),
                covered_weight_percent: covered,
                uncovered_value: (position.shares as f64
                    * position.current_price
                    * (1.0 - covered / 100.0))
                    .max(0.0),
                as_of: own.last().map(|dated| dated.as_of.to_string()),
            }
        })
        .collect()
}

/// 把試算結果封裝成 API 回應。
fn response(
    member_id: i64,
    dimension: TargetDimension,
    etf_coverage: Vec<RebalanceEtfCoverageDto>,
    result: PortfolioRebalanceResult,
) -> PortfolioRebalanceResponse {
    let uncovered_value: f64 = etf_coverage.iter().map(|e| e.uncovered_value).sum();
    PortfolioRebalanceResponse {
        etf_coverage,
        uncovered_value,
        warnings: if uncovered_value > 0.0 {
            vec!["ETF 僅穿透前十大成分，未涵蓋部分不代表零曝險。".to_owned()]
        } else {
            Vec::new()
        },
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
    }
}
