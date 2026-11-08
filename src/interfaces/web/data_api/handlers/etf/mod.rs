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
mod tests;
