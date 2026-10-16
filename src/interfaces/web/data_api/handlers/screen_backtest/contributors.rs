//! 條件選股回測的個股貢獻：整段回測中哪些股票把報酬拉高、哪些拖累最多。
//!
//! 每期的報酬是有報酬持股的加權平均（等權時權重相同），所以一檔股票在該期的貢獻是「這檔報酬 ×
//! 權重 ÷ 當期有報酬持股的權重合計」（百分點）；整段的貢獻是各期貢獻直接相加，不複利、不扣交易
//! 成本，用來看報酬集中在哪幾檔。

use std::collections::HashMap;

use super::portfolio::weight_at;
use super::{PeriodRow, round2};

/// 貢獻最多、拖累最多各列幾檔。
pub(super) const LISTED_CONTRIBUTORS: usize = 10;

/// 一檔股票在整段回測的累計貢獻。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Contribution {
    pub(super) symbol: String,
    /// 持有過的期數（含該期沒有報酬的）。
    pub(super) periods_held: i64,
    /// 各期貢獻相加（百分點），四捨五入到小數兩位。
    pub(super) contribution: f64,
    /// 有報酬那幾期的平均報酬（%）；全部沒有報酬為 `None`。
    pub(super) avg_return: Option<f64>,
}

/// 整段持有過的不同股票數，以及貢獻為正的前 `top` 檔（由大到小）與貢獻為負的前 `top` 檔（由小到大）；
/// 貢獻相同時依代號排序。
pub(super) fn contributors(
    rows: &[PeriodRow],
    top: usize,
) -> (usize, Vec<Contribution>, Vec<Contribution>) {
    // 代號 → (持有期數, 累計貢獻, 報酬合計, 有報酬的期數)
    let mut totals: HashMap<&str, (i64, f64, f64, usize)> = HashMap::new();
    for row in rows {
        let priced: f64 = row
            .returns
            .iter()
            .enumerate()
            .filter(|(_, ret)| ret.is_some())
            .map(|(index, _)| weight_at(row, index))
            .sum();
        for (index, (symbol, ret)) in row.symbols.iter().zip(&row.returns).enumerate() {
            let entry = totals.entry(symbol.as_str()).or_default();
            entry.0 += 1;
            if let Some(ret) = ret {
                entry.1 += ret * weight_at(row, index) / priced;
                entry.2 += ret;
                entry.3 += 1;
            }
        }
    }
    let held = totals.len();
    let mut all: Vec<Contribution> = totals
        .into_iter()
        .map(
            |(symbol, (periods, contribution, sum, priced))| Contribution {
                symbol: symbol.to_owned(),
                periods_held: periods,
                contribution: round2(contribution),
                avg_return: (priced > 0).then(|| round2(sum / priced as f64)),
            },
        )
        .collect();
    all.sort_by(|a, b| {
        b.contribution
            .total_cmp(&a.contribution)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    let best: Vec<Contribution> = all
        .iter()
        .filter(|item| item.contribution > 0.0)
        .take(top)
        .cloned()
        .collect();
    let worst: Vec<Contribution> = all
        .iter()
        .rev()
        .filter(|item| item.contribution < 0.0)
        .take(top)
        .cloned()
        .collect();
    (held, best, worst)
}
