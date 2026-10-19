//! `/strategy-presets/{name}/track` handler：依策略換股通知當天記錄的持股（`strategy_rebalance`），
//! 逐期計算實際表現並與 0050 比較。
//!
//! 每期從換股日收盤持有到下次換股日收盤（最後一期到最新交易日），持股報酬為還原報酬（與目前持股的
//! 「至今報酬」同一算法），策略部位依當天的權重加總、現金報酬 0，再與 0050 核心部位依比例混合；
//! 各期連乘為累積報酬。只看記錄下來的持股，不受之後重跑回測或修改策略條件影響；未扣交易成本。

use std::collections::HashMap;

use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use futures::{StreamExt, stream};

use super::current::return_since;
use super::{ETF_BENCHMARK, round2};
use crate::domain::quote::technical::{PriceAdjustment, PriceBar, PriceBarRepository};
use crate::domain::strategy::StrategyPresetRepository;
use crate::domain::strategy::rebalance::{StrategyRebalanceRecord, StrategyRebalanceRepository};
use crate::infra::database;
use crate::infra::database::repository::{
    quote::PgQuoteRepository, strategy_preset::PgStrategyPresetRepository,
    strategy_rebalance::PgStrategyRebalanceRepository,
};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, StrategyTrackHolding, StrategyTrackPeriod, StrategyTrackResponse,
};
use crate::interfaces::web::data_api::handlers::{database_error, error_response};

/// 同時查詢日線的股票數。
const BAR_CONCURRENCY: usize = 4;

/// 第一次換股日之前多讀幾根日線，換股日剛好沒有成交時往前找最近一根。
const EXTRA_BARS: i64 = 10;

/// 最新交易日（今天含之前）。
const LATEST_TRADING_DATE_SQL: &str =
    r#"SELECT MAX("Date") FROM "DailyQuotes" WHERE "Date" <= CURRENT_DATE"#;

/// 具名策略的實盤追蹤：依換股通知當天記錄的持股逐期計算實際表現（未扣成本），並與 0050 比較。
///
/// # Errors
///
/// 策略不存在且沒有任何換股紀錄回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。策略存在
/// 但還沒有紀錄（尚未遇到換股日）回空的 `periods`。
#[utoipa::path(get, path = "/api/v1/strategy-presets/{name}/track", tag = "data-api", params(("name" = String, Path, description = "策略名稱")), responses((status = 200, body = StrategyTrackResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn strategy_track(Path(name): Path<String>) -> Response {
    let records = match PgStrategyRebalanceRepository::new().list(&name).await {
        Ok(records) => records,
        Err(error) => return internal_error(&error),
    };
    if records.is_empty() {
        return match PgStrategyPresetRepository::new().find(&name).await {
            Ok(Some(_)) => Json(track(
                &name,
                &records,
                None,
                &HashMap::new(),
                &HashMap::new(),
            ))
            .into_response(),
            Ok(None) => error_response(StatusCode::NOT_FOUND, "查無此策略"),
            Err(error) => internal_error(&error),
        };
    }
    let latest: Option<NaiveDate> = match sqlx::query_scalar(LATEST_TRADING_DATE_SQL)
        .fetch_one(database::get_connection())
        .await
    {
        Ok(latest) => latest,
        Err(error) => return database_error(error),
    };
    let (bars, adjustments) = match latest {
        Some(latest) => match load_bars(&records, latest).await {
            Ok(loaded) => loaded,
            Err(error) => return internal_error(&error),
        },
        None => (HashMap::new(), HashMap::new()),
    };
    Json(track(&name, &records, latest, &bars, &adjustments)).into_response()
}

/// 不含細節的 500（細節只記 log）。
fn internal_error(error: &anyhow::Error) -> Response {
    tracing::error!("策略實盤追蹤查詢失敗: {error:?}");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "資料庫查詢失敗")
}

/// 讀取全部持股與 0050 從第一次換股日（往前多讀幾根）到 `latest` 的日線與除權息事件。
async fn load_bars(
    records: &[StrategyRebalanceRecord],
    latest: NaiveDate,
) -> anyhow::Result<(
    HashMap<String, Vec<PriceBar>>,
    HashMap<String, Vec<PriceAdjustment>>,
)> {
    let mut symbols: Vec<String> = records
        .iter()
        .flat_map(|record| record.holdings.iter().map(|holding| holding.symbol.clone()))
        .chain(std::iter::once(ETF_BENCHMARK.to_owned()))
        .collect();
    symbols.sort_unstable();
    symbols.dedup();
    let first = records.first().map_or(latest, |record| record.date);
    let limit = (latest - first).num_days().max(0) + 1 + EXTRA_BARS;
    let repository = PgQuoteRepository::new();
    let adjustments = repository.fetch_price_adjustments(&symbols).await?;
    let repository = &repository;
    let bars: Vec<(String, Vec<PriceBar>)> = stream::iter(symbols)
        .map(|symbol| async move {
            let bars = repository
                .fetch_price_bars(&symbol, Some(latest), limit)
                .await?;
            anyhow::Ok((symbol, bars))
        })
        .buffer_unordered(BAR_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<anyhow::Result<_>>()?;
    Ok((bars.into_iter().collect(), adjustments))
}

/// `symbol` 從 `start` 收盤持有到 `end` 收盤的還原報酬（%）；查不到日線為 `None`。
fn period_return(
    bars: &HashMap<String, Vec<PriceBar>>,
    adjustments: &HashMap<String, Vec<PriceAdjustment>>,
    symbol: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Option<f64> {
    let bars = bars.get(symbol)?;
    let until = bars.partition_point(|bar| bar.date <= end);
    let events = adjustments.get(symbol).map_or(&[][..], Vec::as_slice);
    return_since(&bars[..until], events, start)
}

/// 依換股紀錄逐期計算表現；`latest` 為最新交易日（沒有日線為 `None`）。換股當天的最後一期（還沒有
/// 收盤可比）報酬為 `null`、不列入累積。
pub(super) fn track(
    preset: &str,
    records: &[StrategyRebalanceRecord],
    latest: Option<NaiveDate>,
    bars: &HashMap<String, Vec<PriceBar>>,
    adjustments: &HashMap<String, Vec<PriceAdjustment>>,
) -> StrategyTrackResponse {
    let mut periods = Vec::with_capacity(records.len());
    let mut growth = Some(1.0);
    for (index, record) in records.iter().enumerate() {
        let next = records.get(index + 1).map(|next| next.date);
        let end = match (next, latest) {
            (Some(next), Some(latest)) => next.min(latest),
            (Some(next), None) => next,
            (None, Some(latest)) => latest,
            (None, None) => record.date,
        };
        let started = end > record.date;
        let holdings: Vec<StrategyTrackHolding> = record
            .holdings
            .iter()
            .map(|holding| StrategyTrackHolding {
                symbol: holding.symbol.clone(),
                name: holding.name.clone(),
                weight: holding.weight,
                return_percent: started
                    .then(|| period_return(bars, adjustments, &holding.symbol, record.date, end))
                    .flatten()
                    .map(round2),
            })
            .collect();
        let strategy_return = started
            .then(|| strategy_return(record, bars, adjustments, end))
            .flatten();
        let etf_return = started
            .then(|| period_return(bars, adjustments, ETF_BENCHMARK, record.date, end))
            .flatten();
        let core = f64::from(record.etf_weight) / 100.0;
        let portfolio_return = match (strategy_return, etf_return) {
            (Some(strategy), _) if record.etf_weight == 0 => Some(strategy),
            (Some(strategy), Some(etf)) => Some((1.0 - core) * strategy + core * etf),
            _ => None,
        };
        if started {
            growth = match (growth, portfolio_return) {
                (Some(growth), Some(ret)) => Some(growth * (1.0 + ret / 100.0)),
                _ => None,
            };
        }
        periods.push(StrategyTrackPeriod {
            date: record.date.to_string(),
            end: end.to_string(),
            ongoing: next.is_none(),
            hold_days: record.hold_days,
            etf_weight: record.etf_weight,
            cash: record.cash,
            market_filtered: record.market_filtered,
            holdings,
            strategy_return: strategy_return.map(round2),
            etf_return: etf_return.map(round2),
            portfolio_return: portfolio_return.map(round2),
        });
    }
    let first = records.first().map(|record| record.date);
    let any_started = periods.iter().any(|period| period.date != period.end);
    let cumulative = growth
        .filter(|_| any_started)
        .map(|growth| (growth - 1.0) * 100.0);
    let etf_cumulative = match (first, latest) {
        (Some(first), Some(latest)) if latest > first => {
            period_return(bars, adjustments, ETF_BENCHMARK, first, latest)
        }
        _ => None,
    };
    StrategyTrackResponse {
        preset: preset.to_owned(),
        from: first.map(|date| date.to_string()),
        to: first.and(latest).map(|date| date.to_string()),
        cumulative_return: cumulative.map(round2),
        etf_cumulative_return: etf_cumulative.map(round2),
        excess_return: cumulative
            .zip(etf_cumulative)
            .map(|(strategy, etf)| round2(strategy - etf)),
        periods,
    }
}

/// 策略部位的報酬：有報酬持股依權重加總，查不到的持股以其餘持股的平均代替（權重不變）；現金報酬 0。
/// 沒有持股（空手）為 0；持股都查不到為 `None`。
fn strategy_return(
    record: &StrategyRebalanceRecord,
    bars: &HashMap<String, Vec<PriceBar>>,
    adjustments: &HashMap<String, Vec<PriceAdjustment>>,
    end: NaiveDate,
) -> Option<f64> {
    let total: f64 = record.holdings.iter().map(|holding| holding.weight).sum();
    if record.holdings.is_empty() || total <= 0.0 {
        return Some(0.0);
    }
    let (weighted, known) = record
        .holdings
        .iter()
        .filter_map(|holding| {
            period_return(bars, adjustments, &holding.symbol, record.date, end)
                .map(|ret| (holding.weight * ret, holding.weight))
        })
        .fold((0.0, 0.0), |(sum, weight), (value, w)| {
            (sum + value, weight + w)
        });
    (known > 0.0).then(|| weighted / known * total / 100.0)
}

#[cfg(test)]
mod tests;
