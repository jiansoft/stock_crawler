//! `/etfs/compare` 橫向比較：代號驗證、前十大持股重疊與比較回應。

use super::*;

/// 一次最多比較幾檔。
pub(super) const MAX_COMPARE: usize = 4;

/// 比較 2 至 4 檔 ETF 的費率、規模、折溢價、配息、報酬、風險與前十大持股重疊。
///
/// # Errors
///
/// `symbols` 不是 2 至 4 個不重複的合法代號回 422；其中有不是上市櫃 ETF 的代號回 404；
/// 驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/etfs/compare", tag = "data-api", params(EtfCompareParams), responses((status = 200, body = EtfCompareResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn compare_etfs(Query(params): Query<EtfCompareParams>) -> Response {
    let symbols = match compare_symbols(params.symbols.as_deref()) {
        Ok(symbols) => symbols,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let repo = PgEtfQueryRepository::new();
    let mut details = Vec::with_capacity(symbols.len());
    for symbol in &symbols {
        match crate::infra::cache::etf_analysis::ETF_ANALYSIS
            .detail(symbol, 1, repo.etf_detail(symbol, 1))
            .await
        {
            Ok(Some(detail)) => details.push(detail),
            Ok(None) => {
                return error_response(StatusCode::NOT_FOUND, &format!("查無這檔 ETF：{symbol}"));
            }
            Err(error) => return repository_error(error),
        }
    }
    Json(compare_response(&details)).into_response()
}

/// 解析比較的代號：去空白、轉大寫、2 至 [`MAX_COMPARE`] 檔、不重複、格式合法。
pub(super) fn compare_symbols(value: Option<&str>) -> Result<Vec<String>, &'static str> {
    let symbols: Vec<String> = value
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    if !(2..=MAX_COMPARE).contains(&symbols.len()) {
        return Err("symbols 必須是 2 至 4 檔 ETF 代號，以逗號分隔");
    }
    if symbols.iter().any(|s| !valid_symbol(s)) {
        return Err("ETF 代號格式不正確");
    }
    if (1..symbols.len()).any(|i| symbols[..i].contains(&symbols[i])) {
        return Err("symbols 不可重複");
    }
    Ok(symbols)
}

/// 前十大持股的比對鍵：有台股代號用代號，債券等沒有代號用名稱。
pub(super) fn holding_key(row: &EtfWeightRow) -> &str {
    if row.symbol.is_empty() {
        &row.label
    } else {
        &row.symbol
    }
}

/// 兩檔前十大持股的重疊；任一檔沒有前十大時回 `None`。
pub(super) fn overlap(a: &EtfDetail, b: &EtfDetail) -> Option<EtfOverlap> {
    let holdings = |detail: &'_ EtfDetail| -> Vec<EtfWeightRow> {
        detail
            .weights
            .iter()
            .filter(|w| w.kind == "holding")
            .cloned()
            .collect()
    };
    let (left, right) = (holdings(a), holdings(b));
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let mut common: Vec<(String, f64)> = left
        .iter()
        .filter_map(|l| {
            let r = right.iter().find(|r| holding_key(r) == holding_key(l))?;
            let label = if l.symbol.is_empty() {
                l.label.clone()
            } else {
                format!("{} {}", l.symbol, l.label)
            };
            Some((label, l.weight.min(r.weight)))
        })
        .collect();
    common.sort_by(|x, y| y.1.total_cmp(&x.1));
    Some(EtfOverlap {
        a: a.overview.stock_symbol.clone(),
        b: b.overview.stock_symbol.clone(),
        overlap_weight: round2(common.iter().map(|(_, w)| w).sum()),
        common: common.into_iter().map(|(label, _)| label).collect(),
    })
}

/// 組出比較回應：各檔一列，加上兩兩重疊。
pub(super) fn compare_response(details: &[EtfDetail]) -> EtfCompareResponse {
    let items = details
        .iter()
        .map(|d| {
            let o = &d.overview;
            let p = d.performance.clone().unwrap_or_default();
            let (fee, custodian) = d
                .profile
                .as_ref()
                .map(|p| (p.management_fee.clone(), p.custodian_fee.clone()))
                .unwrap_or_default();
            EtfCompareItem {
                stock_symbol: o.stock_symbol.clone(),
                name: o.name.clone(),
                strategy: o.strategy.clone(),
                benchmark: o.benchmark.clone(),
                management_fee: fee,
                custodian_fee: custodian,
                total_assets: o.total_assets,
                closing_price: o.closing_price.map(round2),
                premium_rate: o.premium_rate.map(round2),
                premium_avg_20d: o.premium_avg_20d.map(round2),
                yield_12m: o.yield_12m.map(round2),
                dividend_frequency: o.dividend_frequency.clone(),
                ret_1y: p.ret_1y.map(round2),
                ret_3y: p.ret_3y.map(round2),
                nav_ret_1y: p.nav_ret_1y.map(round2),
                vol_1y: p.vol_1y.map(round2),
                mdd_1y: p.mdd_1y.map(round2),
                sharpe_1y: p.sharpe_1y.map(round2),
                holdings_as_of: block(&d.weights, "holding").0,
            }
        })
        .collect();
    let overlaps = (0..details.len())
        .flat_map(|i| ((i + 1)..details.len()).map(move |j| (i, j)))
        .filter_map(|(i, j)| overlap(&details[i], &details[j]))
        .collect();
    EtfCompareResponse { items, overlaps }
}
