//! 條件選股回測的持股選擇與交易成本：依上一期持股與 `keep_rank`、續抱門檻、產業上限決定每期持股，
//! 依加權方式決定持股權重，再依換手扣除手續費與證交稅；有設 `etf_weight` 時再與 0050 核心部位混合。

use std::collections::{HashMap, HashSet};

use super::{CandidateRow, PeriodRow, round2, round4};
use crate::interfaces::web::data_api::dto::BacktestPeriod;

/// 手續費率（%），買賣各付一次；未打折。
pub(super) const FEE_RATE: f64 = 0.1425;

/// 股票證交稅率（%），賣出時付。
pub(super) const TAX_RATE: f64 = 0.3;

/// 輸出每期前幾檔代號。
pub(super) const LISTED_SYMBOLS: usize = 10;

/// 持股加權方式（`weighting`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Weighting {
    /// 等權。
    Equal,
    /// 依換股日含當天往前 20 個交易日的平均成交金額（偏向大型股，接近市值加權）。
    TradeValue,
    /// 依平均成交金額的平方根（介於等權與成交金額加權之間）。
    SqrtTradeValue,
}

impl Weighting {
    /// 參數值與加權方式的對照，第一個為預設；字面值與 [`super::sql`] 回測 SQL 的 `$17` 對應。
    pub(super) const ALL: [(&'static str, Self); 3] = [
        ("equal", Self::Equal),
        ("trade_value", Self::TradeValue),
        ("sqrt_trade_value", Self::SqrtTradeValue),
    ];

    /// 參數值轉加權方式；不認得回 `None`。
    pub(super) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, weighting)| *weighting)
    }

    /// 參數值。
    pub(super) fn as_str(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, weighting)| *weighting == self)
            .map_or("equal", |(name, _)| name)
    }
}

/// 持股的相對權重（未正規化）：等權時各為 1；依成交金額加權時為平均成交金額或其平方根，缺值的
/// 股票以其他持股的最小值計，全部缺值時改為等權。
pub(super) fn holding_weights(trade_values: &[Option<f64>], weighting: Weighting) -> Vec<f64> {
    let raw: Vec<Option<f64>> = trade_values
        .iter()
        .map(|value| {
            let value = value.filter(|value| *value > 0.0);
            match weighting {
                Weighting::Equal => Some(1.0),
                Weighting::TradeValue => value,
                Weighting::SqrtTradeValue => value.map(f64::sqrt),
            }
        })
        .collect();
    let floor = raw.iter().flatten().copied().fold(f64::INFINITY, f64::min);
    if !floor.is_finite() {
        return raw.iter().map(|_| 1.0).collect();
    }
    raw.iter().map(|value| value.unwrap_or(floor)).collect()
}

/// 第 `index` 檔持股的相對權重；沒有權重（等權）時為 1。
pub(super) fn weight_at(row: &PeriodRow, index: usize) -> f64 {
    row.weights.get(index).copied().unwrap_or(1.0)
}

/// 依加權方式設定每期持股權重，並把平均報酬改為有報酬持股的加權平均；等權不變。
pub(super) fn weigh(rows: &mut [PeriodRow], weighting: Weighting) {
    if weighting == Weighting::Equal {
        return;
    }
    for row in rows {
        row.weights = holding_weights(&row.trade_values, weighting);
        let (sum, total) = row
            .weights
            .iter()
            .zip(&row.returns)
            .filter_map(|(weight, ret)| ret.map(|ret| (weight * ret, *weight)))
            .fold((0.0, 0.0), |(sum, total), (value, weight)| {
                (sum + value, total + weight)
            });
        row.avg_return = (total > 0.0).then(|| sum / total);
    }
}

/// 一期的換手與扣成本後報酬。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PeriodCost {
    /// 換股時新買進的部位比例（0–1）；空手為 0。
    pub(super) turnover: f64,
    /// 扣除這次換股成本後的報酬（%）。
    pub(super) net_return: f64,
}

/// 依前後兩期的持股算出每期的換手與扣成本後報酬：賣出上一期有、這一期沒有的股票（占上一期的權重），
/// 付手續費與證交稅；買進這一期新增的股票（占這一期的權重），付手續費。第一期全部買進；續抱股票的
/// 權重調整不計成本。
pub(super) fn trading_costs(rows: &[PeriodRow], fee_discount: f64) -> Vec<PeriodCost> {
    let fee = FEE_RATE * fee_discount;
    let mut previous: Option<&PeriodRow> = None;
    rows.iter()
        .map(|row| {
            let share = |from: Option<&PeriodRow>, other: Option<&PeriodRow>| {
                let Some(from) = from else {
                    return 0.0;
                };
                let total: f64 = (0..from.symbols.len())
                    .map(|index| weight_at(from, index))
                    .sum();
                if total <= 0.0 {
                    return 0.0;
                }
                let other: &[String] = other.map_or(&[], |other| other.symbols.as_slice());
                from.symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| !other.contains(symbol))
                    .map(|(index, _)| weight_at(from, index))
                    .sum::<f64>()
                    / total
            };
            let sold = share(previous, Some(row));
            let bought = share(Some(row), previous);
            previous = Some(row);
            PeriodCost {
                turnover: bought,
                net_return: row.avg_return.unwrap_or(0.0) - sold * (fee + TAX_RATE) - bought * fee,
            }
        })
        .collect()
}

/// 把每期報酬換成「`weight` 比例持有 0050、其餘為策略」的組合報酬（`weight` 為 0–1）：扣成本前後的
/// 報酬都是 `weight × 0050 + (1 − weight) × 策略`，換手只算策略部位（乘上 `1 − weight`）。
///
/// 0050 核心部位視為一直持有，不計買賣成本，也不計每期調回目標比例的小額調整；空手（大盤濾網）時
/// 只有策略部位轉成現金，0050 照常持有。0050 缺資料的期數該部位報酬算 0。`weight` 為 0 時不變。
pub(super) fn blend_etf(rows: &mut [PeriodRow], costs: &mut [PeriodCost], weight: f64) {
    if weight <= 0.0 {
        return;
    }
    let mix = |etf: f64, strategy: f64| weight * etf + (1.0 - weight) * strategy;
    for (row, cost) in rows.iter_mut().zip(costs.iter_mut()) {
        let etf = row.etf_return.unwrap_or(0.0);
        row.avg_return = Some(mix(etf, row.avg_return.unwrap_or(0.0)));
        cost.net_return = mix(etf, cost.net_return);
        cost.turnover *= 1.0 - weight;
    }
}

/// 股票池的一檔：代號、報酬、可否續抱、產業編號、平均成交金額。
type PoolEntry<'a> = (&'a str, Option<f64>, bool, i32, Option<f64>);

/// 依候選股決定每期持股：上一期持有、這期仍符合條件且排在前 `keep_rank` 名的股票續抱，
/// 其餘名額依相對強度由高到低補滿 `limit` 檔；`keep_rank` 等於 `limit` 時就是每期取前 `limit` 檔。
///
/// 有設 `hold_min_rs_score` 時，上一期持股即使不再符合選股條件（例如法人連買中斷），只要相對強度
/// 不低於門檻也可續抱；這類股票排在候選股之後，名額不足時先留候選股。
/// 有設 `max_per_industry` 時，同一產業（含續抱）最多 N 檔，排名較後的跳過、由其他產業補上。
/// 大盤濾網沒過的那期空手（上一期持股全部賣出）。平均報酬只算有報酬的持股（之後完全沒有收盤的
/// 股票不列入），全部都沒有報酬為 `None`。
pub(super) fn select_holdings(
    rows: Vec<CandidateRow>,
    limit: usize,
    keep_rank: usize,
    max_per_industry: Option<usize>,
) -> Vec<PeriodRow> {
    let cap = max_per_industry.unwrap_or(usize::MAX);
    let mut previous: HashSet<String> = HashSet::new();
    rows.into_iter()
        .map(|row| {
            let slots = if row.market_ok { limit } else { 0 };
            let holdable: HashSet<&str> = row.hold_symbols.iter().map(String::as_str).collect();
            let listed: HashSet<&str> = row.candidates.iter().map(String::as_str).collect();
            // 股票池：先是依相對強度排序的候選股，再接只符合續抱條件的股票。
            let mut pool: Vec<PoolEntry<'_>> = row
                .candidates
                .iter()
                .zip(&row.candidate_returns)
                .enumerate()
                .map(|(index, (symbol, ret))| {
                    let keepable = index < keep_rank || holdable.contains(symbol.as_str());
                    let industry = row.candidate_industries.get(index).copied().unwrap_or(0);
                    let value = row.candidate_trade_values.get(index).copied().flatten();
                    (symbol.as_str(), *ret, keepable, industry, value)
                })
                .collect();
            pool.extend(
                row.hold_symbols
                    .iter()
                    .zip(&row.hold_returns)
                    .enumerate()
                    .filter(|(_, (symbol, _))| !listed.contains(symbol.as_str()))
                    .map(|(index, (symbol, ret))| {
                        let industry = row.hold_industries.get(index).copied().unwrap_or(0);
                        let value = row.hold_trade_values.get(index).copied().flatten();
                        (symbol.as_str(), *ret, true, industry, value)
                    }),
            );

            // 先放續抱的上一期持股，再依排名補候選股；同產業滿 `cap` 檔就跳過。
            let mut counts: HashMap<i32, usize> = HashMap::new();
            let mut picked: Vec<usize> = Vec::new();
            let mut admit = |index: usize, picked: &mut Vec<usize>| {
                let count = counts.entry(pool[index].3).or_default();
                if *count < cap {
                    *count += 1;
                    picked.push(index);
                }
            };
            for (index, (symbol, _, keepable, _, _)) in pool.iter().enumerate() {
                if picked.len() >= slots {
                    break;
                }
                if *keepable && previous.contains(*symbol) {
                    admit(index, &mut picked);
                }
            }
            for index in 0..row.candidates.len() {
                if picked.len() >= slots {
                    break;
                }
                if !picked.contains(&index) {
                    admit(index, &mut picked);
                }
            }
            picked.sort_unstable();

            let held_returns: Vec<Option<f64>> =
                picked.iter().map(|&index| pool[index].1).collect();
            let returns: Vec<f64> = held_returns.iter().flatten().copied().collect();
            let symbols: Vec<String> = picked
                .iter()
                .map(|&index| pool[index].0.to_owned())
                .collect();
            previous = symbols.iter().cloned().collect();
            PeriodRow {
                date: row.date,
                pending: row.pending,
                market_filtered: !row.market_ok,
                holdings: symbols.len() as i64,
                avg_return: (!returns.is_empty())
                    .then(|| returns.iter().sum::<f64>() / returns.len() as f64),
                benchmark_return: row.benchmark_return,
                etf_return: row.etf_return,
                symbols,
                returns: held_returns,
                trade_values: picked.iter().map(|&index| pool[index].4).collect(),
                weights: Vec::new(),
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
