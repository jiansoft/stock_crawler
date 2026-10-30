//! 每期持股的決定：續抱、補滿、產業上限與大盤濾網。

use chrono::NaiveDate;

use super::*;

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
pub(in super::super) fn select_holdings(
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
                entry_date: row.entry_date,
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

/// 缺平均成交金額的持股（換股日, 代號）：只符合續抱條件、不在本期候選名單的股票，SQL 不查成交金額
/// （見 [`super::super::sql::HOLD_TRADE_VALUES_SQL`]），選定持股後再補查。
pub(in super::super) fn missing_trade_values(rows: &[PeriodRow]) -> Vec<(NaiveDate, String)> {
    let mut missing = Vec::new();
    for row in rows {
        for (index, symbol) in row.symbols.iter().enumerate() {
            if row.trade_values.get(index).copied().flatten().is_none() {
                missing.push((row.date, symbol.clone()));
            }
        }
    }
    missing
}

/// 把補查到的平均成交金額填回持股；查不到（沒有日線）的維持 `None`。
pub(in super::super) fn fill_trade_values(
    rows: &mut [PeriodRow],
    values: &HashMap<(NaiveDate, String), f64>,
) {
    for row in rows {
        let date = row.date;
        for (index, symbol) in row.symbols.iter().enumerate() {
            let Some(slot) = row.trade_values.get_mut(index) else {
                continue;
            };
            if slot.is_none() {
                *slot = values.get(&(date, symbol.clone())).copied();
            }
        }
    }
}
