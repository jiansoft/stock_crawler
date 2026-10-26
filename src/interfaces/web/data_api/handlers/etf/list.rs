//! `/etfs` 清單篩選：分類／配息頻率白名單、參數驗證與清單列組裝。

use super::*;

/// 可用的分類代碼。
pub(super) const STRATEGIES: [&str; 9] = [
    "market_cap",
    "high_dividend",
    "active",
    "bond",
    "leveraged",
    "inverse",
    "commodity",
    "overseas",
    "thematic",
];

/// 可用的配息頻率。
pub(super) const FREQUENCIES: [&str; 5] = ["monthly", "quarterly", "semi_annual", "annual", "none"];

/// 篩選 ETF 清單。
///
/// # Errors
///
/// 參數不合法回 422；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/etfs", tag = "data-api", params(EtfListParams), responses((status = 200, body = EtfListResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn list_etfs(Query(params): Query<EtfListParams>) -> Response {
    let filter = match list_filter(&params) {
        Ok(filter) => filter,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    match PgEtfQueryRepository::new().list_etfs(&filter).await {
        Ok((total, items)) => Json(EtfListResponse {
            total,
            sort: filter.sort.column().to_owned(),
            order: if filter.descending { "desc" } else { "asc" }.to_owned(),
            items: items.iter().map(item).collect(),
        })
        .into_response(),
        Err(error) => repository_error(error),
    }
}

/// 驗證並轉換清單參數。
pub(super) fn list_filter(params: &EtfListParams) -> Result<EtfListFilter, &'static str> {
    let strategies: Vec<String> = params
        .strategy
        .as_deref()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|code| !code.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if strategies
        .iter()
        .any(|code| !STRATEGIES.contains(&code.as_str()))
    {
        return Err(
            "strategy 只能是 market_cap、high_dividend、active、bond、leveraged、inverse、commodity、overseas、thematic",
        );
    }
    let frequency = non_empty(params.frequency.as_deref());
    if frequency
        .as_deref()
        .is_some_and(|value| !FREQUENCIES.contains(&value))
    {
        return Err("frequency 只能是 monthly、quarterly、semi_annual、annual 或 none");
    }
    let region = non_empty(params.region.as_deref());
    if region
        .as_deref()
        .is_some_and(|value| value.chars().count() > 20)
    {
        return Err("region 最多 20 字");
    }
    let keyword = non_empty(params.keyword.as_deref());
    if keyword
        .as_deref()
        .is_some_and(|value| value.chars().count() > 40)
    {
        return Err("keyword 最多 40 字");
    }
    let market_id = match market_id_for_stocks(params.market.as_deref().unwrap_or("all")) {
        Some(0) => None,
        Some(id) => Some(id),
        None => return Err("market 必須為 all、twse 或 tpex"),
    };
    let sort = EtfSort::parse(params.sort.as_deref().unwrap_or("trade_value")).ok_or(
        "sort 只能是 trade_value、total_assets、premium_rate、yield_12m、net_flow、ret_1m、ret_ytd、ret_1y 或 holders",
    )?;
    let descending = match params.order.as_deref().unwrap_or("desc") {
        "desc" => true,
        "asc" => false,
        _ => return Err("order 必須為 desc 或 asc"),
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err("limit 必須介於 1 至 100");
    }
    Ok(EtfListFilter {
        strategies,
        region,
        frequency,
        market_id,
        keyword,
        sort,
        descending,
        limit: i64::from(limit),
    })
}

pub(super) fn item(o: &EtfOverview) -> EtfItem {
    EtfItem {
        stock_symbol: o.stock_symbol.clone(),
        name: o.name.clone(),
        market: if o.market_id == 4 { "tpex" } else { "twse" }.to_owned(),
        strategy: o.strategy.clone(),
        region: o.region.clone(),
        themes: o.themes.clone(),
        dividend_frequency: o.dividend_frequency.clone(),
        issuer: o.issuer.clone(),
        benchmark: o.benchmark.clone(),
        quote_date: date(o.quote_date),
        closing_price: o.closing_price.map(round2),
        change_range: o.change_range.map(round2),
        trade_value: o.trade_value,
        nav_date: date(o.nav_date),
        nav: o.nav,
        premium_rate: o.premium_rate.map(round2),
        premium_avg_20d: o.premium_avg_20d.map(round2),
        total_assets: o.total_assets,
        net_flow: o.net_flow,
        dividend_12m: o.dividend_12m,
        yield_12m: o.yield_12m.map(round2),
        next_ex_date: date(o.next_ex_date),
        next_dividend: o.next_dividend,
        ret_1m: o.ret_1m.map(round2),
        ret_ytd: o.ret_ytd.map(round2),
        ret_1y: o.ret_1y.map(round2),
        holders: o.holders,
        foreign_net: o.foreign_net,
    }
}
