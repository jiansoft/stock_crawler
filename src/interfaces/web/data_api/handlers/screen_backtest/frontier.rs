//! `/stocks/screen/backtest/frontier` handler：同一組條件搭配不同比例的 0050 核心部位，看報酬與
//! 風險怎麼變化。
//!
//! 0050 核心部位只是把每期報酬混合（見 [`super::portfolio::blend_etf`]），不影響選股，所以每個
//! 換股日偏移只查一次候選股，再依 [`WEIGHTS`] 各混合一次；每個比例回傳各偏移績效的分布（與
//! `/stocks/screen/backtest/offsets` 相同），並標出夏普值中位數最高、年化報酬中位數最高的比例。
//! 比例 100% 就是只持有 0050（同樣的換股日進出、不計成本）。

use axum::{
    Json,
    extract::Query,
    response::{IntoResponse, Response},
};
use futures::{StreamExt, TryStreamExt, stream};

use super::offsets::{CONCURRENCY, distribution, sample_offsets};
use super::summary::summarize;
use super::{Backtest, ETF_BENCHMARK, Raw};
use crate::interfaces::web::data_api::dto::{
    BacktestFrontierPoint, BacktestOffsetDistribution, BacktestOffsetResult, ErrorBody,
    ScreenBacktestFrontierResponse, ScreenBacktestParams,
};
use crate::interfaces::web::data_api::handlers::database_error;

/// 要比較的 0050 核心部位比例（%）：0（純策略）到 100（只持有 0050），每 10% 一點。
pub(super) const WEIGHTS: [u8; 11] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

/// 同一組條件搭配 0%～100% 的 0050 核心部位，回傳每個比例在各換股日偏移下的績效分布。
///
/// 參數與 `/stocks/screen/backtest` 相同，`etf_weight` 與 `rebalance_offset` 不使用：換股日偏移的
/// 取法與 `/stocks/screen/backtest/offsets` 相同。
///
/// # Errors
///
/// 參數驗證與 `/stocks/screen/backtest` 相同（422）；迄日之前沒有相對強度資料回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest/frontier", tag = "data-api", params(ScreenBacktestParams), responses((status = 200, body = ScreenBacktestFrontierResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest_frontier(
    Query(params): Query<ScreenBacktestParams>,
) -> Response {
    let backtest = match Backtest::prepare(&params).await {
        Ok(backtest) => backtest,
        Err(response) => return *response,
    };
    let backtest = &backtest;
    let raws: Vec<(u16, Raw)> = match stream::iter(sample_offsets(backtest.hold_days))
        .map(|offset| async move { backtest.run(offset).await.map(|raw| (offset, raw)) })
        .buffered(CONCURRENCY)
        .try_collect()
        .await
    {
        Ok(raws) => raws,
        Err(error) => return database_error(error),
    };
    let points = frontier(&raws, backtest.hold_days);

    Json(ScreenBacktestFrontierResponse {
        from: backtest.from.to_string(),
        to: backtest.to.to_string(),
        market: backtest.market.to_owned(),
        hold_days: backtest.hold_days,
        limit: backtest.limit,
        rank_by: backtest.rank_by.to_owned(),
        weighting: backtest.weighting.as_str().to_owned(),
        fee_discount: backtest.fee_discount,
        etf_benchmark: ETF_BENCHMARK.to_owned(),
        offsets: raws.len() as i64,
        best_sharpe_weight: best_weight(&points, |spread| {
            spread.sharpe_ratio.as_ref().map(|stat| stat.median)
        }),
        best_return_weight: best_weight(&points, |spread| {
            spread
                .annualized_return_after_costs
                .as_ref()
                .map(|stat| stat.median)
        }),
        weights: points,
    })
    .into_response()
}

/// 每個 [`WEIGHTS`] 比例：各偏移混合 0050 後彙總，再算分布。
pub(super) fn frontier(raws: &[(u16, Raw)], hold_days: u16) -> Vec<BacktestFrontierPoint> {
    WEIGHTS
        .iter()
        .map(|&etf_weight| {
            let results: Vec<BacktestOffsetResult> = raws
                .iter()
                .map(|(offset, raw)| {
                    let simulation = raw.clone().finish(etf_weight);
                    BacktestOffsetResult {
                        rebalance_offset: *offset,
                        summary: summarize(&simulation.rows, &simulation.costs, hold_days),
                    }
                })
                .collect();
            BacktestFrontierPoint {
                etf_weight,
                distribution: distribution(&results),
            }
        })
        .collect()
}

/// `metric` 中位數最高的比例；同值取比例較低（策略部位較多）的，全部沒有數值為 `None`。
pub(super) fn best_weight(
    points: &[BacktestFrontierPoint],
    metric: impl Fn(&BacktestOffsetDistribution) -> Option<f64>,
) -> Option<u8> {
    points
        .iter()
        .filter_map(|point| metric(&point.distribution).map(|value| (point.etf_weight, value)))
        .fold(
            None,
            |best: Option<(u8, f64)>, (weight, value)| match best {
                Some((_, top)) if top >= value => best,
                _ => Some((weight, value)),
            },
        )
        .map(|(weight, _)| weight)
}
