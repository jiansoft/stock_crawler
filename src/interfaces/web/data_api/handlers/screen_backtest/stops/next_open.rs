//! 次日開盤停損：收盤觸發後尋找下一個有效開盤，成交不得跨越原定出場日。

use super::*;

/// 一次停損的實際成交日與相對進場開盤的還原報酬。
#[derive(Debug, Clone, Copy)]
pub(super) struct StopExit {
    /// 真正有有效開盤的賣出日。
    pub(super) date: NaiveDate,
    /// 買進開盤至停損賣出開盤的報酬（%），未扣費稅。
    pub(super) ret: f64,
}

/// 使用收盤更新峰值與觸發停損；停牌或缺開盤則順延，到達期末時由原定期末出場接手。
pub(super) fn stop_exit(
    series: &[PriceBar],
    symbol: &str,
    start: NaiveDate,
    end: NaiveDate,
    rule: StopRule,
    ratios: &OpenRatios,
) -> Option<StopExit> {
    if !rule.is_active() || start >= end {
        return None;
    }
    let at = series.partition_point(|bar| bar.date <= start);
    let entry = series.get(at.checked_sub(1)?)?.close
        * ratios
            .get(&(symbol.to_owned(), start))
            .copied()
            .unwrap_or(1.0);
    if !entry.is_finite() || entry <= 0.0 {
        return None;
    }
    let from = series.partition_point(|bar| bar.date < start);
    let mut peak = entry;
    let mut triggered = false;
    for bar in series[from..].iter().take_while(|bar| bar.date < end) {
        if !bar.close.is_finite() || bar.close <= 0.0 {
            continue;
        }
        // 先處理前一交易日已觸發的委託；當天開盤賣出後不能再使用當天收盤判斷。
        if triggered {
            if let Some(ratio) = ratios.get(&(symbol.to_owned(), bar.date)) {
                let open = bar.close * ratio;
                if open.is_finite() && open > 0.0 {
                    return Some(StopExit {
                        date: bar.date,
                        ret: (open / entry - 1.0) * 100.0,
                    });
                }
            }
            continue;
        }
        peak = peak.max(bar.close);
        triggered = rule
            .stop_loss
            .is_some_and(|limit| (bar.close / entry - 1.0) * 100.0 <= -limit)
            || rule
                .trailing_stop
                .is_some_and(|limit| (bar.close / peak - 1.0) * 100.0 <= -limit);
    }
    None
}

/// 將已完成期數的持股報酬改為停損實際開盤報酬；費用交由既有交易成本函式統一扣除。
pub(in super::super) fn apply_next_open_stops(
    rows: &mut [PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
    rule: StopRule,
    ratios: &OpenRatios,
) {
    for row in rows.iter_mut().filter(|row| !row.pending) {
        let (Some(start), Some(end)) = (row.entry_date, row.exit_date) else {
            continue;
        };
        let mut stopped = vec![false; row.symbols.len()];
        for (index, symbol) in row.symbols.iter().enumerate() {
            if row.returns.get(index).copied().flatten().is_none() {
                continue;
            }
            if let Some(exit) = bars
                .get(symbol)
                .and_then(|series| stop_exit(series, symbol, start, end, rule, ratios))
            {
                row.returns[index] = Some(exit.ret);
                stopped[index] = true;
            }
        }
        if stopped.contains(&true) {
            let returns: Vec<_> = row.returns.iter().flatten().copied().collect();
            row.avg_return = Some(returns.iter().sum::<f64>() / returns.len() as f64);
            row.stopped = stopped;
        }
    }
}

/// 批次讀取持有窗口內所有有效開盤比值，供停損成交順延判斷；不把缺開盤當成收盤賣出。
pub(in super::super) async fn load_stop_open_ratios(
    rows: &[PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
) -> Result<OpenRatios, sqlx::Error> {
    let mut points = HashSet::new();
    for row in rows.iter().filter(|row| !row.pending) {
        let (Some(start), Some(end)) = (row.entry_date, row.exit_date) else {
            continue;
        };
        for symbol in &row.symbols {
            if let Some(series) = bars.get(symbol) {
                let from = series.partition_point(|bar| bar.date < start);
                points.extend(
                    series[from..]
                        .iter()
                        .take_while(|bar| bar.date <= end)
                        .map(|bar| (symbol.clone(), bar.date)),
                );
            }
        }
    }
    PgQuoteRepository::new()
        .fetch_open_ratios(&points.into_iter().collect::<Vec<_>>())
        .await
}

#[cfg(test)]
mod tests;
