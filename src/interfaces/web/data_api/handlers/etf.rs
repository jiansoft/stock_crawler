//! `/etfs`、`/etfs/{symbol}` handler：ETF 專區的清單篩選與單檔明細。
//!
//! 資料由排程預先整理（`etf_snapshot`、`etf_performance`、`etf_profile`、持股與淨值），見
//! [`crate::infra::database::repository::etf_query`]。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

use super::{error_response, market_id_for_stocks, repository_error, round2};
use crate::domain::etf::query::{
    EtfDetail, EtfListFilter, EtfOverview, EtfPerformanceView, EtfProfileView, EtfQueryRepository,
    EtfSort, EtfWeightRow,
};
use crate::infra::database::repository::etf_query::PgEtfQueryRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, EtfAllocationItem, EtfCompareItem, EtfCompareParams, EtfCompareResponse,
    EtfDetailParams, EtfDetailResponse, EtfHoldingItem, EtfItem, EtfListParams, EtfListResponse,
    EtfNavPoint, EtfOverlap, EtfPerformanceInfo, EtfProfileInfo,
};

/// 可用的分類代碼。
const STRATEGIES: [&str; 9] = [
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
const FREQUENCIES: [&str; 5] = ["monthly", "quarterly", "semi_annual", "annual", "none"];

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

/// 一次最多比較幾檔。
const MAX_COMPARE: usize = 4;

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
        match repo.etf_detail(symbol, 1).await {
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
fn compare_symbols(value: Option<&str>) -> Result<Vec<String>, &'static str> {
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
fn holding_key(row: &EtfWeightRow) -> &str {
    if row.symbol.is_empty() {
        &row.label
    } else {
        &row.symbol
    }
}

/// 兩檔前十大持股的重疊；任一檔沒有前十大時回 `None`。
fn overlap(a: &EtfDetail, b: &EtfDetail) -> Option<EtfOverlap> {
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
fn compare_response(details: &[EtfDetail]) -> EtfCompareResponse {
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

/// 驗證並轉換清單參數。
fn list_filter(params: &EtfListParams) -> Result<EtfListFilter, &'static str> {
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

/// 去掉前後空白；空字串視為沒有提供。
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// ETF 代號只有英數字，最多 12 碼。
pub(super) fn valid_symbol(symbol: &str) -> bool {
    !symbol.is_empty() && symbol.len() <= 12 && symbol.chars().all(|c| c.is_ascii_alphanumeric())
}

fn date(value: Option<NaiveDate>) -> Option<String> {
    value.map(|date| date.to_string())
}

fn item(o: &EtfOverview) -> EtfItem {
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

fn profile(p: EtfProfileView) -> EtfProfileInfo {
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

fn performance(p: EtfPerformanceView) -> EtfPerformanceInfo {
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
fn block<'a>(weights: &'a [EtfWeightRow], kind: &str) -> (Option<String>, Vec<&'a EtfWeightRow>) {
    let rows: Vec<&EtfWeightRow> = weights.iter().filter(|w| w.kind == kind).collect();
    (rows.first().map(|w| w.as_of.to_string()), rows)
}

fn allocations(rows: Vec<&EtfWeightRow>) -> Vec<EtfAllocationItem> {
    rows.into_iter()
        .map(|w| EtfAllocationItem {
            label: w.label.clone(),
            weight: round2(w.weight),
        })
        .collect()
}

/// 組出明細回應。
fn detail_response(detail: EtfDetail) -> EtfDetailResponse {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::etf::query::EtfNavDay;

    #[test]
    fn list_filter_validates_and_defaults() {
        let filter = list_filter(&EtfListParams::default()).expect("預設參數合法");
        assert_eq!(
            (
                filter.sort,
                filter.descending,
                filter.limit,
                filter.market_id
            ),
            (EtfSort::TradeValue, true, 20, None)
        );
        let filter = list_filter(&EtfListParams {
            strategy: Some(" bond, high_dividend ,".to_owned()),
            region: Some(" 美國 ".to_owned()),
            frequency: Some("monthly".to_owned()),
            market: Some("tpex".to_owned()),
            keyword: Some("  ".to_owned()),
            sort: Some("yield_12m".to_owned()),
            order: Some("asc".to_owned()),
            limit: Some(5),
        })
        .expect("合法參數");
        assert_eq!(filter.strategies, vec!["bond", "high_dividend"]);
        assert_eq!(
            (filter.region.as_deref(), filter.keyword, filter.market_id),
            (Some("美國"), None, Some(4))
        );
        assert_eq!((filter.sort, filter.descending), (EtfSort::Yield12m, false));

        for bad in [
            EtfListParams {
                strategy: Some("bond,stock".to_owned()),
                ..Default::default()
            },
            EtfListParams {
                frequency: Some("weekly".to_owned()),
                ..Default::default()
            },
            EtfListParams {
                region: Some("台".repeat(21)),
                ..Default::default()
            },
            EtfListParams {
                keyword: Some("字".repeat(41)),
                ..Default::default()
            },
            EtfListParams {
                market: Some("nyse".to_owned()),
                ..Default::default()
            },
            EtfListParams {
                sort: Some("name".to_owned()),
                ..Default::default()
            },
            EtfListParams {
                order: Some("up".to_owned()),
                ..Default::default()
            },
            EtfListParams {
                limit: Some(0),
                ..Default::default()
            },
        ] {
            assert!(list_filter(&bad).is_err(), "{bad:?} 應為 422");
        }
    }

    #[test]
    fn compare_symbols_normalize_and_validate() {
        assert_eq!(
            compare_symbols(Some(" 0050, 006208 ,00679b,")),
            Ok(vec![
                "0050".to_owned(),
                "006208".to_owned(),
                "00679B".to_owned()
            ])
        );
        for bad in [
            None,
            Some("0050"),
            Some("1,2,3,4,5"),
            Some("0050,0050"),
            Some("0050,00;"),
        ] {
            assert!(compare_symbols(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn compare_response_lists_items_and_pairwise_overlap() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).expect("日期合法");
        let holding = |symbol: &str, label: &str, weight: f64| EtfWeightRow {
            kind: "holding".to_owned(),
            as_of: day,
            symbol: symbol.to_owned(),
            label: label.to_owned(),
            weight,
        };
        let detail = |symbol: &str, weights: Vec<EtfWeightRow>| EtfDetail {
            overview: EtfOverview {
                stock_symbol: symbol.to_owned(),
                premium_rate: Some(0.456),
                ..Default::default()
            },
            weights,
            ..Default::default()
        };
        let details = vec![
            detail(
                "0050",
                vec![
                    holding("2330", "台積電", 56.4),
                    holding("2317", "鴻海", 4.8),
                    holding("2454", "聯發科", 4.5),
                ],
            ),
            EtfDetail {
                profile: Some(EtfProfileView {
                    management_fee: "0.15%".to_owned(),
                    ..Default::default()
                }),
                performance: Some(EtfPerformanceView {
                    nav_ret_1y: Some(73.891),
                    ..Default::default()
                }),
                ..detail(
                    "006208",
                    vec![
                        holding("2330", "台積電", 57.0),
                        holding("2454", "聯發科", 4.2),
                    ],
                )
            },
            // 債券 ETF 沒有台股持股；與其他檔沒有共同持股時仍列出 0。
            detail("00679B", vec![holding("", "US TREASURY 2055", 5.3)]),
            // 沒有前十大持股的不列重疊。
            detail("00632R", Vec::new()),
        ];
        let response = compare_response(&details);
        assert_eq!(response.items.len(), 4);
        assert_eq!(response.items[0].premium_rate, Some(0.46));
        assert_eq!(
            response.items[0].holdings_as_of.as_deref(),
            Some("2026-10-01")
        );
        assert_eq!(response.items[1].management_fee, "0.15%");
        assert_eq!(response.items[1].nav_ret_1y, Some(73.89));
        assert_eq!(response.overlaps.len(), 3, "00632R 沒有持股不列");
        let first = &response.overlaps[0];
        assert_eq!((first.a.as_str(), first.b.as_str()), ("0050", "006208"));
        assert_eq!(first.common, vec!["2330 台積電", "2454 聯發科"]);
        assert_eq!(first.overlap_weight, 60.6);
        assert_eq!(response.overlaps[1].overlap_weight, 0.0);
        assert!(response.overlaps[1].common.is_empty());
    }

    #[test]
    fn symbol_must_be_short_alphanumeric() {
        assert!(valid_symbol("00679B") && valid_symbol("0050"));
        assert!(!valid_symbol("") && !valid_symbol("0050;") && !valid_symbol("0123456789ABC"));
    }

    #[test]
    fn detail_response_splits_blocks_and_rounds() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 21).expect("日期合法");
        let weight = |kind: &str, label: &str, weight: f64| EtfWeightRow {
            kind: kind.to_owned(),
            as_of: day,
            symbol: if kind == "holding" { "2330" } else { "" }.to_owned(),
            label: label.to_owned(),
            weight,
        };
        let response = detail_response(EtfDetail {
            overview: EtfOverview {
                stock_symbol: "0050".to_owned(),
                market_id: 2,
                premium_rate: Some(0.4567),
                quote_date: Some(day),
                ..Default::default()
            },
            profile: Some(EtfProfileView {
                full_name: "元大台灣卓越50".to_owned(),
                inception_date: Some(day),
                ..Default::default()
            }),
            performance: Some(EtfPerformanceView {
                ret_1y: Some(73.919),
                nav_calc_date: Some(day),
                nav_ret_1y: Some(73.891),
                ..Default::default()
            }),
            weights: vec![
                weight("holding", "台積電", 56.456),
                weight("industry", "半導體業", 68.461),
                weight("asset", "股票", 99.454),
            ],
            navs: vec![EtfNavDay {
                date: day,
                nav: Some(108.27),
                market_price: Some(108.15),
                premium_rate: Some(-0.1108),
                units_change: None,
            }],
        });
        assert_eq!(response.item.market, "twse");
        assert_eq!(response.item.premium_rate, Some(0.46));
        assert_eq!(response.item.quote_date.as_deref(), Some("2026-10-21"));
        assert_eq!(
            response.profile.map(|p| p.inception_date),
            Some(Some("2026-10-21".to_owned()))
        );
        let perf = response.performance.expect("有績效");
        assert_eq!((perf.ret_1y, perf.nav_ret_1y), (Some(73.92), Some(73.89)));
        assert_eq!(response.holdings.len(), 1);
        assert_eq!(
            (
                response.holdings[0].symbol.as_str(),
                response.holdings[0].weight
            ),
            ("2330", 56.46)
        );
        assert_eq!(response.holdings_as_of.as_deref(), Some("2026-10-21"));
        assert_eq!(response.industries[0].label, "半導體業");
        assert_eq!(response.assets[0].weight, 99.45);
        assert_eq!(response.navs[0].premium_rate, Some(-0.11));

        let bare = detail_response(EtfDetail {
            overview: EtfOverview {
                market_id: 4,
                ..Default::default()
            },
            ..Default::default()
        });
        assert_eq!(bare.item.market, "tpex");
        assert!(bare.holdings_as_of.is_none() && bare.profile.is_none() && bare.navs.is_empty());
    }
}
