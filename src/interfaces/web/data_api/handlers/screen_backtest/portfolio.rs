//! 條件選股回測的持股選擇與交易成本：依上一期持股與 `keep_rank`、續抱門檻、產業上限決定每期持股，
//! 依加權方式決定持股權重，再依換手扣除手續費與證交稅；有設 `etf_weight` 時再與 0050 核心部位混合。

use std::collections::{HashMap, HashSet};

use super::{CandidateRow, PeriodRow, round2, round4};
use crate::interfaces::web::data_api::dto::BacktestPeriod;

/// 手續費率（%），買賣各付一次；未打折。
pub(super) const FEE_RATE: f64 = crate::domain::portfolio::costs::FEE_RATE * 100.0;

/// 股票證交稅率（%），賣出時付。
pub(super) const TAX_RATE: f64 = crate::domain::portfolio::costs::STOCK_TAX_RATE * 100.0;

/// ETF 證交稅率（%），賣出 0050 時付。
pub(super) const ETF_TAX_RATE: f64 = crate::domain::portfolio::costs::ETF_TAX_RATE * 100.0;

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

/// 把相對權重正規化並套用單檔上限 `cap`（0–1）：超過上限的持股固定在上限，多出的部分依比例分給
/// 其他持股，重複到沒有持股超過上限；全部都到上限仍分不完（檔數 × 上限 < 1）時剩下的放現金。
/// 回傳正規化後的權重（合計為 1 − 現金）與現金比例；沒有持股時現金為 0。
pub(super) fn cap_weights(weights: &[f64], cap: f64) -> (Vec<f64>, f64) {
    let mut capped = vec![false; weights.len()];
    loop {
        let fixed = capped.iter().filter(|capped| **capped).count() as f64 * cap;
        let free: f64 = weights
            .iter()
            .zip(&capped)
            .filter(|(_, capped)| !**capped)
            .map(|(weight, _)| weight)
            .sum();
        let share = |weight: f64| {
            if free > 0.0 {
                weight / free * (1.0 - fixed)
            } else {
                0.0
            }
        };
        let mut changed = false;
        for (weight, capped) in weights.iter().zip(capped.iter_mut()) {
            if !*capped && share(*weight) > cap + 1e-12 {
                *capped = true;
                changed = true;
            }
        }
        if !changed {
            let result: Vec<f64> = weights
                .iter()
                .zip(&capped)
                .map(|(weight, capped)| if *capped { cap } else { share(*weight) })
                .collect();
            let cash = if result.is_empty() {
                0.0
            } else {
                (1.0 - result.iter().sum::<f64>()).max(0.0)
            };
            return (result, cash);
        }
    }
}

/// 依加權方式與單檔上限 `max_weight`（0–1）設定每期持股權重與現金比例，並把平均報酬改為有報酬
/// 持股的加權平均乘上持股比例（現金報酬為 0）；等權且沒有上限時不變。
pub(super) fn weigh(rows: &mut [PeriodRow], weighting: Weighting, max_weight: Option<f64>) {
    if weighting == Weighting::Equal && max_weight.is_none() {
        return;
    }
    for row in rows {
        let values: Vec<Option<f64>> = (0..row.symbols.len())
            .map(|index| row.trade_values.get(index).copied().flatten())
            .collect();
        let weights = holding_weights(&values, weighting);
        (row.weights, row.cash) = match max_weight {
            Some(cap) => cap_weights(&weights, cap),
            None => (weights, 0.0),
        };
        let (sum, total) = row
            .weights
            .iter()
            .zip(&row.returns)
            .filter_map(|(weight, ret)| ret.map(|ret| (weight * ret, *weight)))
            .fold((0.0, 0.0), |(sum, total), (value, weight)| {
                (sum + value, total + weight)
            });
        row.avg_return = (total > 0.0).then(|| sum / total * (1.0 - row.cash));
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

/// 持股占策略部位的比例（合計為 1 − 現金）；沒有持股為空。
pub(super) fn shares(row: &PeriodRow) -> Vec<f64> {
    let weights: Vec<f64> = (0..row.symbols.len())
        .map(|index| weight_at(row, index))
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return weights.iter().map(|_| 0.0).collect();
    }
    weights
        .iter()
        .map(|weight| weight / total * (1.0 - row.cash))
        .collect()
}

/// 一期結束、下次換股前各持股占策略部位的比例：期初比例依各檔報酬漲跌後重新換算（連同報酬為 0
/// 的現金合計為 1）。沒有報酬的持股（之後沒有收盤）視為與該期持股的平均報酬同幅。期間停損賣出的
/// 持股已轉成現金，比例為 0（出場時的價值仍計入合計）。
fn drifted(row: &PeriodRow) -> Vec<f64> {
    let start = shares(row);
    let invested = 1.0 - row.cash;
    let average = if invested > 0.0 {
        row.avg_return.unwrap_or(0.0) / invested
    } else {
        0.0
    };
    let grown: Vec<f64> = start
        .iter()
        .enumerate()
        .map(|(index, share)| {
            let ret = row.returns.get(index).copied().flatten().unwrap_or(average);
            share * (1.0 + ret / 100.0).max(0.0)
        })
        .collect();
    let total: f64 = grown.iter().sum::<f64>() + row.cash;
    if total <= 0.0 {
        return start;
    }
    grown
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if is_stopped(row, index) {
                0.0
            } else {
                value / total
            }
        })
        .collect()
}

/// 第 `index` 檔持股這期是否在期間停損賣出。
fn is_stopped(row: &PeriodRow, index: usize) -> bool {
    row.stopped.get(index).copied().unwrap_or(false)
}

/// 期間停損賣出的部位占期初策略部位的比例：各停損持股的期初比例 × (1 + 出場報酬)；沒有停損為 0。
pub(super) fn stopped_value(row: &PeriodRow) -> f64 {
    shares(row)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| is_stopped(row, *index))
        .map(|(index, share)| {
            let ret = row.returns.get(index).copied().flatten().unwrap_or(0.0);
            share * (1.0 + ret / 100.0).max(0.0)
        })
        .sum()
}

/// 依前後兩期的持股算出每期的換手與扣成本後報酬：上一期持股漲跌後的比例調到這一期的目標比例，
/// 減碼（含整檔賣出）的部分付手續費與證交稅，加碼（含新買進）的部分付手續費。第一期全部買進；
/// 空手時賣光。續抱股票的再平衡（例如等權持股漲多的減碼、依成交金額加權時權重變動）也計入。
/// 期間停損賣出的部位（見 [`stopped_value`]）在當期付手續費與證交稅，下次換股若仍入選視為重新買進。
pub(super) fn trading_costs(rows: &[PeriodRow], fee_discount: f64) -> Vec<PeriodCost> {
    let fee = FEE_RATE * fee_discount;
    // 上一期期末各持股的比例。
    let mut previous: HashMap<&str, f64> = HashMap::new();
    rows.iter()
        .map(|row| {
            let mut bought = 0.0;
            let mut sold = 0.0;
            for (symbol, target) in row.symbols.iter().zip(shares(row)) {
                let before = previous.remove(symbol.as_str()).unwrap_or(0.0);
                if target > before {
                    bought += target - before;
                } else {
                    sold += before - target;
                }
            }
            sold += previous.values().sum::<f64>();
            previous = row
                .symbols
                .iter()
                .map(String::as_str)
                .zip(drifted(row))
                .collect();
            PeriodCost {
                turnover: bought,
                net_return: row.avg_return.unwrap_or(0.0)
                    - (sold + stopped_value(row)) * (fee + TAX_RATE)
                    - bought * fee,
            }
        })
        .collect()
}

/// 把每期報酬換成「`weight` 比例持有 0050、其餘為策略」的組合報酬（`weight` 為 0–1）：扣成本前的
/// 報酬是 `weight × 0050 + (1 − weight) × 策略`，扣成本後另扣每期換股時把組合調回固定比例的成本；
/// 策略部位自己的換手乘上 `1 − weight`。
///
/// 上一期 0050 漲得比策略多，換股時賣出多出的 0050（手續費與 ETF 證交稅 0.1%）、加碼策略部位
/// （手續費）；反之減碼策略部位（手續費與證交稅）、買進 0050（手續費）。策略部位上一期空手時減碼
/// 的是現金、這一期空手時加碼的是現金，不付成本。0050 第一次買進不計成本，與只持有 0050 的基準
/// 一致；比例 100% 時沒有調整，等於只持有 0050。空手（大盤濾網）時只有策略部位轉成現金，0050 照常
/// 持有。0050 缺資料的期數該部位報酬算 0。`weight` 為 0 時不變。
pub(super) fn blend_etf(
    rows: &mut [PeriodRow],
    costs: &mut [PeriodCost],
    weight: f64,
    fee_discount: f64,
) {
    if weight <= 0.0 {
        return;
    }
    let fee = FEE_RATE * fee_discount;
    let mix = |etf: f64, strategy: f64| weight * etf + (1.0 - weight) * strategy;
    // 上一期的 0050 報酬、策略報酬（扣成本前）與策略部位是否有持股。
    let mut previous: Option<(f64, f64, bool)> = None;
    for (row, cost) in rows.iter_mut().zip(costs.iter_mut()) {
        let etf = row.etf_return.unwrap_or(0.0);
        let strategy = row.avg_return.unwrap_or(0.0);
        let invested = row.holdings > 0;
        let (rebalance, rebalance_cost) = match previous {
            Some((last_etf, last_strategy, last_invested)) => {
                let etf_value = weight * (1.0 + last_etf / 100.0).max(0.0);
                let total = etf_value + (1.0 - weight) * (1.0 + last_strategy / 100.0).max(0.0);
                let drift = if total > 0.0 {
                    etf_value / total - weight
                } else {
                    0.0
                };
                let amount = drift.abs();
                // 0050 多了：賣 0050、加碼策略；少了：減碼策略、買 0050。策略部位是現金的那一側不付
                // 成本；換手只算買進股票或 0050 的部分。
                let (etf_rate, strategy_rate, bought) = if drift > 0.0 {
                    let rate = if invested { fee } else { 0.0 };
                    (
                        fee + ETF_TAX_RATE,
                        rate,
                        if invested { amount } else { 0.0 },
                    )
                } else {
                    let rate = if last_invested { fee + TAX_RATE } else { 0.0 };
                    (fee, rate, amount)
                };
                (bought, amount * (etf_rate + strategy_rate))
            }
            None => (0.0, 0.0),
        };
        previous = Some((etf, strategy, invested));
        // 核心與衛星必須混合同日淨值；100% 核心不依賴任何策略日線。
        if row.etf_daily_complete && (weight >= 1.0 || row.daily_complete) {
            let len = row.etf_daily_returns.len();
            row.daily_returns = (0..len)
                .map(|i| {
                    mix(
                        row.etf_daily_returns[i],
                        if weight >= 1.0 {
                            0.0
                        } else {
                            row.daily_returns[i]
                        },
                    )
                })
                .collect();
            row.daily_net_returns = (0..len)
                .map(|i| {
                    mix(
                        row.etf_daily_returns[i],
                        if weight >= 1.0 {
                            0.0
                        } else {
                            row.daily_net_returns[i]
                        },
                    ) - rebalance_cost
                })
                .collect();
            row.daily_complete = true;
        } else {
            row.daily_complete = false;
        }
        row.avg_return = Some(mix(etf, strategy));
        cost.net_return = mix(etf, cost.net_return) - rebalance_cost;
        cost.turnover = cost.turnover * (1.0 - weight) + rebalance;
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
                exit_date: row.exit_date,
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
                cash: 0.0,
                stopped: Vec::new(),
                daily_returns: Vec::new(),
                daily_net_returns: Vec::new(),
                etf_daily_returns: Vec::new(),
                daily_complete: false,
                etf_daily_complete: false,
                industries: picked.iter().map(|&index| pool[index].3).collect(),
            }
        })
        .collect()
}

/// 一期的輸出：報酬四捨五入到小數兩位；代號只列前 [`LISTED_SYMBOLS`] 檔。
pub(super) fn period((mut row, cost): (PeriodRow, PeriodCost)) -> BacktestPeriod {
    row.symbols.truncate(LISTED_SYMBOLS);
    BacktestPeriod {
        stopped: super::summary::stop_count(&row),
        date: row.date.to_string(),
        exit_date: row.exit_date.map(|date| date.to_string()),
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
