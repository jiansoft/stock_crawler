//! 條件選股回測的持股選擇與交易成本：依上一期持股與 `keep_rank`、續抱門檻決定每期持股，
//! 再依換手扣除手續費與證交稅。

use std::collections::HashSet;

use super::{CandidateRow, PeriodRow, round2, round4};
use crate::interfaces::web::data_api::dto::BacktestPeriod;

/// 手續費率（%），買賣各付一次；未打折。
pub(super) const FEE_RATE: f64 = 0.1425;

/// 股票證交稅率（%），賣出時付。
pub(super) const TAX_RATE: f64 = 0.3;

/// 輸出每期前幾檔代號。
pub(super) const LISTED_SYMBOLS: usize = 10;

/// 一期的換手與扣成本後報酬。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PeriodCost {
    /// 換股時新買進的部位比例（0–1）；空手為 0。
    pub(super) turnover: f64,
    /// 扣除這次換股成本後的報酬（%）。
    pub(super) net_return: f64,
}

/// 依前後兩期的持股算出每期的換手與扣成本後報酬：賣出上一期有、這一期沒有的股票（占上一期的比例），
/// 付手續費與證交稅；買進這一期新增的股票（占這一期的比例），付手續費。第一期全部買進。
pub(super) fn trading_costs(rows: &[PeriodRow], fee_discount: f64) -> Vec<PeriodCost> {
    let fee = FEE_RATE * fee_discount;
    let mut previous: &[String] = &[];
    rows.iter()
        .map(|row| {
            let current = row.symbols.as_slice();
            let share = |from: &[String], other: &[String]| {
                if from.is_empty() {
                    0.0
                } else {
                    from.iter().filter(|symbol| !other.contains(symbol)).count() as f64
                        / from.len() as f64
                }
            };
            let sold = share(previous, current);
            let bought = share(current, previous);
            previous = current;
            PeriodCost {
                turnover: bought,
                net_return: row.avg_return.unwrap_or(0.0) - sold * (fee + TAX_RATE) - bought * fee,
            }
        })
        .collect()
}

/// 依候選股決定每期持股：上一期持有、這期仍符合條件且排在前 `keep_rank` 名的股票續抱，
/// 其餘名額依相對強度由高到低補滿 `limit` 檔；`keep_rank` 等於 `limit` 時就是每期取前 `limit` 檔。
///
/// 有設 `hold_min_rs_score` 時，上一期持股即使不再符合選股條件（例如法人連買中斷），只要相對強度
/// 不低於門檻也可續抱；這類股票排在候選股之後，名額不足時先留候選股。
/// 大盤濾網沒過的那期空手（上一期持股全部賣出）。平均報酬只算有報酬的持股（之後完全沒有收盤的
/// 股票不列入），全部都沒有報酬為 `None`。
pub(super) fn select_holdings(
    rows: Vec<CandidateRow>,
    limit: usize,
    keep_rank: usize,
) -> Vec<PeriodRow> {
    let mut previous: HashSet<String> = HashSet::new();
    rows.into_iter()
        .map(|row| {
            let slots = if row.market_ok { limit } else { 0 };
            let holdable: HashSet<&str> = row.hold_symbols.iter().map(String::as_str).collect();
            let listed: HashSet<&str> = row.candidates.iter().map(String::as_str).collect();
            // 股票池：先是依相對強度排序的候選股，再接只符合續抱條件的股票；第三欄為可否續抱。
            let mut pool: Vec<(&str, Option<f64>, bool)> = row
                .candidates
                .iter()
                .zip(&row.candidate_returns)
                .enumerate()
                .map(|(index, (symbol, ret))| {
                    let keepable = index < keep_rank || holdable.contains(symbol.as_str());
                    (symbol.as_str(), *ret, keepable)
                })
                .collect();
            pool.extend(
                row.hold_symbols
                    .iter()
                    .zip(&row.hold_returns)
                    .filter(|(symbol, _)| !listed.contains(symbol.as_str()))
                    .map(|(symbol, ret)| (symbol.as_str(), *ret, true)),
            );

            let mut picked: Vec<usize> = pool
                .iter()
                .enumerate()
                .filter(|(_, (symbol, _, keepable))| *keepable && previous.contains(*symbol))
                .map(|(index, _)| index)
                .take(slots)
                .collect();
            for index in 0..row.candidates.len() {
                if picked.len() >= slots {
                    break;
                }
                if !picked.contains(&index) {
                    picked.push(index);
                }
            }
            picked.sort_unstable();

            let returns: Vec<f64> = picked.iter().filter_map(|&index| pool[index].1).collect();
            let symbols: Vec<String> = picked
                .iter()
                .map(|&index| pool[index].0.to_owned())
                .collect();
            previous = symbols.iter().cloned().collect();
            PeriodRow {
                date: row.date,
                market_filtered: !row.market_ok,
                holdings: symbols.len() as i64,
                avg_return: (!returns.is_empty())
                    .then(|| returns.iter().sum::<f64>() / returns.len() as f64),
                benchmark_return: row.benchmark_return,
                etf_return: row.etf_return,
                symbols,
            }
        })
        .collect()
}

/// 一期的輸出：報酬四捨五入到小數兩位；代號只列前 [`LISTED_SYMBOLS`] 檔。
pub(super) fn period((mut row, cost): (PeriodRow, PeriodCost)) -> BacktestPeriod {
    row.symbols.truncate(LISTED_SYMBOLS);
    BacktestPeriod {
        date: row.date.to_string(),
        holdings: row.holdings,
        market_filtered: row.market_filtered,
        avg_return: row.avg_return.map(round2),
        return_after_costs: round2(cost.net_return),
        turnover: round4(cost.turnover),
        benchmark_return: row.benchmark_return.map(round2),
        etf_return: row.etf_return.map(round2),
        symbols: row.symbols,
    }
}
