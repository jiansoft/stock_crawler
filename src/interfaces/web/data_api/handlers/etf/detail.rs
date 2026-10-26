//! `/etfs/{symbol}` 單檔明細：基本資料、績效、持股與資產配置。

use super::*;

/// 查詢單檔 ETF 的概況、基本資料、績效、持股配置與近期淨值。
///
/// # Errors
///
/// `nav_days` 超出 1–60 或代號格式不合法回 422；不是上市櫃 ETF 回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/etfs/{symbol}", tag = "data-api", params(("symbol" = String, Path, description = "ETF 代號（例如 0050、00679B）"), EtfDetailParams), responses((status = 200, body = EtfDetailResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn etf_detail(
    Path(symbol): Path<String>,
    Query(params): Query<EtfDetailParams>,
) -> Response {
    let nav_days = params.nav_days.unwrap_or(20);
    if !(1..=60).contains(&nav_days) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "nav_days 必須介於 1 至 60",
        );
    }
    if !valid_symbol(&symbol) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "ETF 代號格式不正確");
    }
    match PgEtfQueryRepository::new()
        .etf_detail(&symbol, i64::from(nav_days))
        .await
    {
        Ok(Some(detail)) => Json(detail_response(detail)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無這檔 ETF"),
        Err(error) => repository_error(error),
    }
}

pub(super) fn profile(p: EtfProfileView) -> EtfProfileInfo {
    EtfProfileInfo {
        full_name: p.full_name,
        inception_date: date(p.inception_date),
        is_active: p.is_active,
        management_fee: p.management_fee,
        custodian_fee: p.custodian_fee,
        currency: p.currency,
        source: p.source,
    }
}

pub(super) fn performance(p: EtfPerformanceView) -> EtfPerformanceInfo {
    EtfPerformanceInfo {
        calc_date: date(p.calc_date),
        ret_1w: p.ret_1w.map(round2),
        ret_1m: p.ret_1m.map(round2),
        ret_3m: p.ret_3m.map(round2),
        ret_6m: p.ret_6m.map(round2),
        ret_ytd: p.ret_ytd.map(round2),
        ret_1y: p.ret_1y.map(round2),
        ret_3y: p.ret_3y.map(round2),
        ret_5y: p.ret_5y.map(round2),
        vol_1y: p.vol_1y.map(round2),
        mdd_1y: p.mdd_1y.map(round2),
        beta_1y: p.beta_1y.map(round2),
        sharpe_1y: p.sharpe_1y.map(round2),
        nav_calc_date: date(p.nav_calc_date),
        nav_ret_1m: p.nav_ret_1m.map(round2),
        nav_ret_ytd: p.nav_ret_ytd.map(round2),
        nav_ret_1y: p.nav_ret_1y.map(round2),
        nav_ret_3y: p.nav_ret_3y.map(round2),
    }
}

/// 某一區塊的資料日期與列（依倉儲排序）。
pub(super) fn block<'a>(
    weights: &'a [EtfWeightRow],
    kind: &str,
) -> (Option<String>, Vec<&'a EtfWeightRow>) {
    let rows: Vec<&EtfWeightRow> = weights.iter().filter(|w| w.kind == kind).collect();
    (rows.first().map(|w| w.as_of.to_string()), rows)
}

pub(super) fn allocations(rows: Vec<&EtfWeightRow>) -> Vec<EtfAllocationItem> {
    rows.into_iter()
        .map(|w| EtfAllocationItem {
            label: w.label.clone(),
            weight: round2(w.weight),
        })
        .collect()
}

/// 組出明細回應。
pub(super) fn detail_response(detail: EtfDetail) -> EtfDetailResponse {
    let (holdings_as_of, holdings) = block(&detail.weights, "holding");
    let (industries_as_of, industries) = block(&detail.weights, "industry");
    let (assets_as_of, assets) = block(&detail.weights, "asset");
    EtfDetailResponse {
        item: item(&detail.overview),
        profile: detail.profile.map(profile),
        performance: detail.performance.map(performance),
        holdings_as_of,
        holdings: holdings
            .into_iter()
            .map(|w| EtfHoldingItem {
                symbol: w.symbol.clone(),
                name: w.label.clone(),
                weight: round2(w.weight),
            })
            .collect(),
        industries_as_of,
        industries: allocations(industries),
        assets_as_of,
        assets: allocations(assets),
        navs: detail
            .navs
            .iter()
            .map(|n| EtfNavPoint {
                date: n.date.to_string(),
                nav: n.nav,
                market_price: n.market_price,
                premium_rate: n.premium_rate.map(round2),
                units_change: n.units_change,
            })
            .collect(),
    }
}
