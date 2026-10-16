//! 條件選股回測的目前持股：最後一次換股、還沒滿持有天數的持股，相對上一期的新買進與賣出，
//! 換股日收盤到迄日的還原報酬，以及個股貢獻與目前持股用的股票名稱。

use std::collections::HashMap;

use chrono::NaiveDate;
use futures::{StreamExt, stream};

use super::portfolio::{self, PeriodCost};
use super::{ETF_BENCHMARK, PeriodRow, round2, round4};
use crate::domain::quote::technical::{PriceAdjustment, PriceBar, PriceBarRepository, adjust_bars};
use crate::infra::database;
use crate::infra::database::repository::quote::PgQuoteRepository;
use crate::interfaces::web::data_api::dto::{BacktestCurrent, BacktestExit, BacktestHolding};

/// 同時查詢日線的股票數。
const RETURN_CONCURRENCY: usize = 4;

/// 換股日之前多讀幾根日線，換股日剛好沒有成交時往前找最近一根。
const EXTRA_BARS: i64 = 10;

/// 最後一期還沒滿持有天數時把它從各期結果移出，回傳該期持股與換手；其餘期數不受影響。
pub(super) fn split_current(
    rows: &mut Vec<PeriodRow>,
    costs: &mut Vec<PeriodCost>,
) -> Option<(PeriodRow, PeriodCost)> {
    if !rows.last()?.pending {
        return None;
    }
    Some((rows.pop()?, costs.pop()?))
}

/// 個股貢獻與目前持股的股票名稱。
const STOCK_NAMES_SQL: &str =
    r#"SELECT stock_symbol::text, "Name"::text FROM stocks WHERE stock_symbol = ANY($1)"#;

/// 股票代號 → 名稱；查無的代號不列。
pub(super) async fn stock_names(symbols: &[&str]) -> Result<HashMap<String, String>, sqlx::Error> {
    let names: Vec<(String, String)> = sqlx::query_as(STOCK_NAMES_SQL)
        .bind(symbols)
        .fetch_all(database::get_connection())
        .await?;
    Ok(names.into_iter().collect())
}

/// 上一期持有、這一期沒有的股票與上一期換股時的權重（%）；沒有上一期或上一期空手為空。
pub(super) fn exited(row: &PeriodRow, previous: Option<&PeriodRow>) -> Vec<(String, f64)> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    previous
        .symbols
        .iter()
        .zip(portfolio::shares(previous))
        .filter(|(symbol, _)| !row.symbols.contains(symbol))
        .map(|(symbol, share)| (symbol.clone(), round2(share * 100.0)))
        .collect()
}

/// 換股日（含之前最近一根）收盤到最後一根收盤的還原報酬（%）；`bars` 由舊到新，沒有換股日當天或
/// 之前的日線、或價格不正時回 `None`。
pub(super) fn return_since(
    bars: &[PriceBar],
    adjustments: &[PriceAdjustment],
    since: NaiveDate,
) -> Option<f64> {
    let adjusted = adjust_bars(bars, adjustments);
    let base = adjusted.iter().rev().find(|bar| bar.date <= since)?;
    let last = adjusted.last()?;
    (base.close > 0.0 && last.close > 0.0).then(|| (last.close / base.close - 1.0) * 100.0)
}

/// 目前持股與 [`ETF_BENCHMARK`] 從換股日收盤到 `until` 的還原報酬（%）。只是附加資訊：查詢失敗的
/// 股票記 warning 後略過。
pub(super) async fn returns_to_date(
    row: &PeriodRow,
    until: NaiveDate,
    elapsed: i64,
) -> HashMap<String, f64> {
    let symbols: Vec<String> = row
        .symbols
        .iter()
        .cloned()
        .chain(std::iter::once(ETF_BENCHMARK.to_owned()))
        .collect();
    let repository = PgQuoteRepository::new();
    let adjustments = match repository.fetch_price_adjustments(&symbols).await {
        Ok(adjustments) => adjustments,
        Err(error) => {
            tracing::warn!("目前持股報酬：讀取除權息事件失敗，略過：{error:?}");
            return HashMap::new();
        }
    };
    let limit = elapsed.max(0) + 1 + EXTRA_BARS;
    let repository = &repository;
    let adjustments = &adjustments;
    stream::iter(symbols)
        .map(|symbol| async move {
            match repository
                .fetch_price_bars(&symbol, Some(until), limit)
                .await
            {
                Ok(bars) => {
                    let events = adjustments.get(&symbol).map_or(&[][..], Vec::as_slice);
                    return_since(&bars, events, row.date).map(|ret| (symbol, ret))
                }
                Err(error) => {
                    tracing::warn!("目前持股報酬：讀取 {symbol} 日線失敗，略過：{error:?}");
                    None
                }
            }
        })
        .buffer_unordered(RETURN_CONCURRENCY)
        .filter_map(|item| async move { item })
        .collect()
        .await
}

/// 持股依權重的報酬：有報酬持股的加權平均乘上持股比例（現金報酬為 0）；全部沒有報酬為 `None`。
pub(super) fn weighted_return(row: &PeriodRow, returns: &HashMap<String, f64>) -> Option<f64> {
    let (sum, total) = row
        .symbols
        .iter()
        .zip(portfolio::shares(row))
        .filter_map(|(symbol, share)| returns.get(symbol).map(|ret| (share * ret, share)))
        .fold((0.0, 0.0), |(sum, total), (value, share)| {
            (sum + value, total + share)
        });
    (total > 0.0).then(|| sum / total * (1.0 - row.cash))
}

/// 最後一次換股、還沒滿持有天數的持股：各檔權重、是否為新買進、換股日至今的報酬（`returns`，見
/// [`returns_to_date`]），以及相對上一期賣出的股票。
pub(super) fn current_holdings(
    (row, cost): (PeriodRow, PeriodCost),
    previous: Option<&PeriodRow>,
    elapsed: i64,
    hold_days: u16,
    names: &HashMap<String, String>,
    returns: &HashMap<String, f64>,
) -> BacktestCurrent {
    let held: Vec<&str> = previous
        .map(|previous| previous.symbols.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let exits = exited(&row, previous);
    let weights: Vec<f64> = portfolio::shares(&row)
        .into_iter()
        .map(|share| round2(share * 100.0))
        .collect();
    let return_to_date = weighted_return(&row, returns).map(round2);
    BacktestCurrent {
        return_to_date,
        etf_return_to_date: returns.get(ETF_BENCHMARK).copied().map(round2),
        date: row.date.to_string(),
        days_elapsed: elapsed,
        days_to_next_rebalance: (i64::from(hold_days) - elapsed).max(0),
        market_filtered: row.market_filtered,
        turnover: round4(cost.turnover),
        cash: round2(row.cash * 100.0),
        holdings: row
            .symbols
            .into_iter()
            .zip(weights)
            .map(|(symbol, weight)| BacktestHolding {
                name: names.get(&symbol).cloned(),
                added: !held.contains(&symbol.as_str()),
                return_to_date: returns.get(&symbol).copied().map(round2),
                symbol,
                weight,
            })
            .collect(),
        exited: exits
            .into_iter()
            .map(|(symbol, weight)| BacktestExit {
                name: names.get(&symbol).cloned(),
                symbol,
                weight,
            })
            .collect(),
    }
}
