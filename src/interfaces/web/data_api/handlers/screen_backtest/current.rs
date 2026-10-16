//! 條件選股回測的目前持股：最後一次換股、還沒滿持有天數的持股，相對上一期的新買進與賣出，
//! 以及個股貢獻與目前持股用的股票名稱。

use std::collections::HashMap;

use super::portfolio::{self, PeriodCost};
use super::{PeriodRow, round2, round4};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{BacktestCurrent, BacktestExit, BacktestHolding};

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

/// 最後一次換股、還沒滿持有天數的持股：各檔權重、是否為新買進，以及相對上一期賣出的股票。
pub(super) fn current_holdings(
    (row, cost): (PeriodRow, PeriodCost),
    previous: Option<&PeriodRow>,
    elapsed: i64,
    hold_days: u16,
    names: &HashMap<String, String>,
) -> BacktestCurrent {
    let held: Vec<&str> = previous
        .map(|previous| previous.symbols.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let exits = exited(&row, previous);
    let weights: Vec<f64> = portfolio::shares(&row)
        .into_iter()
        .map(|share| round2(share * 100.0))
        .collect();
    BacktestCurrent {
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
