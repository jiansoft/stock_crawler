//! `/etfs/{symbol}/holdings/history` handler：ETF 近幾期的前十大持股與期間差異。
//!
//! 每期持股由週六的 Yahoo 採集寫入 `etf_holding_history`（收到新一期才新增），差異以
//! [`diff_holdings`] 計算：多取一期，最舊那期也能和更早一期比較。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rust_decimal::prelude::ToPrimitive;

use super::{error_response, repository_error, round2};
use crate::domain::etf::{
    EtfHoldingPeriod,
    holding_diff::{HoldingWeight, diff_holdings},
    query::{EtfHoldingHistory, EtfQueryRepository},
};
use crate::infra::database::repository::etf_query::PgEtfQueryRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, EtfHoldingChanges, EtfHoldingHistoryParams, EtfHoldingHistoryResponse,
    EtfHoldingItem, EtfHoldingMoveItem, EtfHoldingPeriodItem, EtfHoldingWeightItem,
};

/// 預設回傳幾期。
const DEFAULT_PERIODS: u8 = 6;

/// 最多回傳幾期。
const MAX_PERIODS: u8 = 24;

/// 查詢一檔 ETF 近幾期的前十大持股，與每一期相對前一期的新進、剔除與權重變化。
///
/// # Errors
///
/// `periods` 超出 1–24 或代號格式不合法回 422；不是上市櫃 ETF 回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/etfs/{symbol}/holdings/history", tag = "data-api", params(("symbol" = String, Path, description = "ETF 代號（例如 0050、00878）"), EtfHoldingHistoryParams), responses((status = 200, body = EtfHoldingHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn etf_holding_history(
    Path(symbol): Path<String>,
    Query(params): Query<EtfHoldingHistoryParams>,
) -> Response {
    let periods = params.periods.unwrap_or(DEFAULT_PERIODS);
    if !(1..=MAX_PERIODS).contains(&periods) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "periods 必須介於 1 至 24");
    }
    let symbol = symbol.trim().to_ascii_uppercase();
    if !super::etf::valid_symbol(&symbol) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "ETF 代號格式不正確");
    }
    match PgEtfQueryRepository::new()
        .holding_history(&symbol, i64::from(periods) + 1)
        .await
    {
        Ok(Some(history)) => Json(history_response(history, usize::from(periods))).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無這檔 ETF"),
        Err(error) => repository_error(error),
    }
}

fn weight_item(weight: HoldingWeight) -> EtfHoldingWeightItem {
    EtfHoldingWeightItem {
        label: weight.label,
        weight: round2(weight.weight),
    }
}

/// 這一期與前一期的差異。
fn changes(previous: &EtfHoldingPeriod, current: &EtfHoldingPeriod) -> EtfHoldingChanges {
    let diff = diff_holdings(&previous.holdings, &current.holdings);
    EtfHoldingChanges {
        previous_as_of: previous.as_of.to_string(),
        added: diff.added.into_iter().map(weight_item).collect(),
        removed: diff.removed.into_iter().map(weight_item).collect(),
        moved: diff
            .moved
            .into_iter()
            .map(|moved| EtfHoldingMoveItem {
                label: moved.label,
                before: round2(moved.before),
                after: round2(moved.after),
            })
            .collect(),
    }
}

/// 組出回應：只回傳最新 `periods` 期，多取的那一期只用來算最舊那期的差異。
fn history_response(history: EtfHoldingHistory, periods: usize) -> EtfHoldingHistoryResponse {
    let items = history
        .periods
        .iter()
        .enumerate()
        .take(periods)
        .map(|(index, period)| EtfHoldingPeriodItem {
            as_of: period.as_of.to_string(),
            holdings: period
                .holdings
                .iter()
                .map(|record| EtfHoldingItem {
                    symbol: record.holding_symbol.clone(),
                    name: record.holding_name.clone(),
                    weight: round2(record.weight.to_f64().unwrap_or_default()),
                })
                .collect(),
            changes: history
                .periods
                .get(index + 1)
                .map(|previous| changes(previous, period)),
        })
        .collect();
    EtfHoldingHistoryResponse {
        stock_symbol: history.stock_symbol,
        name: history.name,
        periods: items,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use super::*;
    use crate::domain::etf::EtfHoldingRecord;

    fn record(rank: i16, symbol: &str, name: &str, weight: &str) -> EtfHoldingRecord {
        EtfHoldingRecord {
            rank,
            holding_symbol: symbol.to_owned(),
            holding_name: name.to_owned(),
            weight: weight.parse::<Decimal>().expect("比例合法"),
        }
    }

    fn period(month: u32, holdings: Vec<EtfHoldingRecord>) -> EtfHoldingPeriod {
        EtfHoldingPeriod {
            as_of: NaiveDate::from_ymd_opt(2026, month, 1).expect("日期合法"),
            holdings,
        }
    }

    #[test]
    fn each_period_lists_changes_against_the_one_before() {
        let history = EtfHoldingHistory {
            stock_symbol: "0056".to_owned(),
            name: "元大高股息".to_owned(),
            periods: vec![
                period(
                    10,
                    vec![
                        record(1, "2454", "聯發科", "8.456"),
                        record(2, "2303", "聯電", "4.0"),
                    ],
                ),
                period(
                    9,
                    vec![
                        record(1, "2454", "聯發科", "7.5"),
                        record(2, "2412", "中華電", "3.2"),
                    ],
                ),
                // 多取的一期：只用來算 9 月的差異，不回傳。
                period(8, vec![record(1, "2454", "聯發科", "7.5")]),
            ],
        };
        let response = history_response(history, 2);
        assert_eq!(response.periods.len(), 2);
        let latest = &response.periods[0];
        assert_eq!(latest.as_of, "2026-10-01");
        assert_eq!(latest.holdings[0].weight, 8.46);
        let changes = latest.changes.as_ref().expect("有前一期");
        assert_eq!(changes.previous_as_of, "2026-09-01");
        assert_eq!(changes.added[0].label, "2303 聯電");
        assert_eq!(changes.removed[0].label, "2412 中華電");
        assert_eq!(
            (changes.moved[0].before, changes.moved[0].after),
            (7.5, 8.46)
        );
        let september = response.periods[1].changes.as_ref().expect("有 8 月");
        assert_eq!(september.added[0].label, "2412 中華電");
        assert!(september.moved.is_empty() && september.removed.is_empty());
    }

    #[test]
    fn the_oldest_period_without_history_has_no_changes() {
        let history = EtfHoldingHistory {
            periods: vec![period(10, vec![record(1, "", "US TREASURY 2055", "5")])],
            ..Default::default()
        };
        let response = history_response(history, 6);
        assert_eq!(response.periods.len(), 1);
        assert_eq!(response.periods[0].holdings[0].symbol, "");
        assert!(response.periods[0].changes.is_none());
        assert!(
            history_response(EtfHoldingHistory::default(), 6)
                .periods
                .is_empty()
        );
    }
}
