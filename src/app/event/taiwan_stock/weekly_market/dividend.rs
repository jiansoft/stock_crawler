//! 每週市場摘要的持股填息一段：近 7 天內填息的持股，與尚未填息、離填息價最遠的幾檔。
//!
//! 填息規則與 `/portfolio/dividend-fill`、17:20 填息通知相同（[`fill::recent_outcomes`]），只看近
//! [`LOOKBACK_DAYS`] 天內的除權息。

use chrono::{Days, NaiveDate};

use super::TOP;
use crate::domain::dividend::fill::{self, DividendFillQuery, FillOutcome};

/// 只看除權息日在今天往前這麼多天內的事件。
const LOOKBACK_DAYS: u64 = 365;

/// 「本週填息」往前看的天數（週六執行，涵蓋週一到週五）。
const RECENT_FILL_DAYS: u64 = 7;

/// 一檔持股的一次除權息與填息結果。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoldingFill {
    pub(super) stock_symbol: String,
    pub(super) outcome: FillOutcome,
}

/// 讀取持股近 [`LOOKBACK_DAYS`] 天內每次除權息的填息結果；單檔失敗只記錄警告。
pub(super) async fn holding_fills(
    query: &impl DividendFillQuery,
    symbols: impl IntoIterator<Item = &String>,
    today: NaiveDate,
) -> Vec<HoldingFill> {
    let since = today - Days::new(LOOKBACK_DAYS);
    let mut fills = Vec::new();
    for symbol in symbols {
        match fill::recent_outcomes(query, symbol, since, today).await {
            Ok(outcomes) => fills.extend(outcomes.into_iter().map(|outcome| HoldingFill {
                stock_symbol: symbol.clone(),
                outcome,
            })),
            Err(why) => tracing::warn!("每週市場摘要：讀取 {symbol} 填息失敗: {why:#}"),
        }
    }
    fills
}

/// 持股填息一段的各行（開頭為空行）；近一年沒有除權息時回空陣列。
pub(super) fn lines(
    fills: &[HoldingFill],
    today: NaiveDate,
    stock_name: &impl Fn(&str) -> String,
) -> Vec<String> {
    if fills.is_empty() {
        return Vec::new();
    }
    let filled = fills
        .iter()
        .filter(|fill| fill.outcome.fill_date.is_some())
        .count();
    let mut lines = vec![String::new()];
    lines.push(format!(
        "持股填息（近一年除權息 {} 次，已填息 {filled} 次）",
        fills.len()
    ));
    let recent_from = today - Days::new(RECENT_FILL_DAYS);
    let mut recent: Vec<&HoldingFill> = fills
        .iter()
        .filter(|fill| {
            fill.outcome
                .fill_date
                .is_some_and(|date| date > recent_from && date <= today)
        })
        .collect();
    recent.sort_by_key(|fill| (fill.outcome.fill_date, fill.stock_symbol.clone()));
    if !recent.is_empty() {
        let text: Vec<String> = recent
            .iter()
            .map(|fill| {
                format!(
                    "{} {}（{} 除權息，第 {} 個交易日）",
                    fill.stock_symbol,
                    stock_name(&fill.stock_symbol),
                    fill.outcome.event.ex_date.format("%m-%d"),
                    fill.outcome.trading_days_to_fill.unwrap_or_default()
                )
            })
            .collect();
        lines.push(format!("• 本週填息：{}", text.join("、")));
    }
    let mut unfilled: Vec<&HoldingFill> = fills
        .iter()
        .filter(|fill| fill.outcome.fill_date.is_none())
        .collect();
    unfilled.sort_by(|a, b| {
        b.outcome
            .gap_percent()
            .cmp(&a.outcome.gap_percent())
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    if !unfilled.is_empty() {
        let text: Vec<String> = unfilled
            .iter()
            .take(TOP)
            .map(|fill| {
                format!(
                    "{} {} 差 {:.1}%（{} 除權息，已 {} 個交易日）",
                    fill.stock_symbol,
                    stock_name(&fill.stock_symbol),
                    fill.outcome.gap_percent().unwrap_or_default(),
                    fill.outcome.event.ex_date.format("%m-%d"),
                    fill.outcome.trading_days_since_ex
                )
            })
            .collect();
        lines.push(format!(
            "• 尚未填息 {} 次，差最多：{}",
            unfilled.len(),
            text.join("、")
        ));
    }
    lines
}
