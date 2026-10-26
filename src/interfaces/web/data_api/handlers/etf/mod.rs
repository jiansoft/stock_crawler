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

mod compare;
mod detail;
mod list;

// 子模組以 `use super::*` 互相取用彼此的輔助函式。
use self::{detail::*, list::*};
pub(crate) use compare::{__path_compare_etfs, compare_etfs};
pub(crate) use detail::{__path_etf_detail, etf_detail};
pub(crate) use list::{__path_list_etfs, list_etfs};

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

#[cfg(test)]
mod tests {
    use super::compare::*;
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
