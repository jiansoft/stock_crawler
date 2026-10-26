//! 條件選股回測的績效彙總：累積與年化報酬、最大回撤、勝率、波動率與夏普值，以及依年度的彙總。

use chrono::Datelike;

use super::{PeriodCost, PeriodRow, round2, round4};
use crate::interfaces::web::data_api::dto::{BacktestSummary, BacktestYear};

/// 每年約有幾個交易日，用來把每期報酬換算成年化報酬。
pub(super) const TRADING_DAYS_PER_YEAR: f64 = 245.0;

/// 累積淨值（起始 1）走過各期報酬（%）後的最終淨值與最大回撤（比例，負值或 0）。
pub(super) fn compound(returns: impl IntoIterator<Item = f64>) -> (f64, f64) {
    let mut equity = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut drawdown = 0.0_f64;
    for percent in returns {
        equity *= 1.0 + percent / 100.0;
        peak = peak.max(equity);
        drawdown = drawdown.min(equity / peak - 1.0);
    }
    (equity, drawdown)
}

/// 以各期期末報酬與各期期內日頻報酬序列，計算持有期內日頻最大回撤（負值或 0）。
///
/// 若某期有 `daily_returns`，則在該期內逐日走訪日累積淨值：
/// `day_equity = period_start_equity * (1.0 + day_ret / 100.0)`，
/// 更新歷史最高峰 `peak` 與期間最大回撤。
/// 期末時淨值結算為 `period_start_equity * (1.0 + period_end_return / 100.0)`。
/// 若某期無 `daily_returns`，則直接走訪期末淨值。
/// 因此，日頻最大回撤之幅度保證大於或等於期末最大回撤（|daily_dd| >= |period_end_dd|）。
pub(super) fn compound_daily<'a>(periods: impl IntoIterator<Item = (f64, &'a [f64])>) -> f64 {
    let mut equity = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut max_dd = 0.0_f64;
    for (period_end_return, daily_returns) in periods {
        let period_start_equity = equity;
        for &day_ret in daily_returns {
            let day_equity = period_start_equity * (1.0 + day_ret / 100.0);
            peak = peak.max(day_equity);
            if peak > 0.0 {
                max_dd = max_dd.min(day_equity / peak - 1.0);
            }
        }
        equity = period_start_equity * (1.0 + period_end_return / 100.0);
        peak = peak.max(equity);
        if peak > 0.0 {
            max_dd = max_dd.min(equity / peak - 1.0);
        }
    }
    max_dd
}

/// 這期在期間停損賣出的檔數。
pub(super) fn stop_count(row: &PeriodRow) -> i64 {
    row.stopped.iter().filter(|stopped| **stopped).count() as i64
}

/// 彙總各期：空手的期數報酬算 0；年化以每年約 [`TRADING_DAYS_PER_YEAR`] 個交易日換算；
/// `costs` 與 `rows` 一一對應（見 [`super::portfolio::trading_costs`]）。
pub(super) fn summarize(
    rows: &[PeriodRow],
    costs: &[PeriodCost],
    hold_days: u16,
) -> BacktestSummary {
    let periods = rows.len();
    let invested: Vec<&PeriodRow> = rows.iter().filter(|row| row.holdings > 0).collect();
    let returns: Vec<f64> = rows
        .iter()
        .map(|row| row.avg_return.unwrap_or(0.0))
        .collect();
    let benchmarks: Vec<f64> = rows
        .iter()
        .map(|row| row.benchmark_return.unwrap_or(0.0))
        .collect();
    let etf: Vec<f64> = rows
        .iter()
        .map(|row| row.etf_return.unwrap_or(0.0))
        .collect();
    let has_etf = rows.iter().any(|row| row.etf_return.is_some());
    if periods == 0 {
        return BacktestSummary {
            periods: 0,
            invested_periods: 0,
            market_filtered_periods: 0,
            stop_exits: 0,
            etf_missing_periods: 0,
            avg_holdings: None,
            avg_turnover: None,
            total_return: None,
            total_return_after_costs: None,
            benchmark_total_return: None,
            annualized_return: None,
            annualized_return_after_costs: None,
            benchmark_annualized_return: None,
            max_drawdown: None,
            max_drawdown_after_costs: None,
            period_end_max_drawdown: None,
            period_end_max_drawdown_after_costs: None,
            daily_max_drawdown: None,
            daily_max_drawdown_after_costs: None,
            benchmark_max_drawdown: None,
            win_rate: None,
            beat_market_rate: None,
            avg_period_return: None,
            avg_benchmark_return: None,
            etf_total_return: None,
            etf_annualized_return: None,
            etf_max_drawdown: None,
            beat_etf_rate: None,
            volatility: None,
            sharpe_ratio: None,
            benchmark_volatility: None,
            benchmark_sharpe_ratio: None,
            etf_volatility: None,
            etf_sharpe_ratio: None,
        };
    }
    let per_year = TRADING_DAYS_PER_YEAR / f64::from(hold_days);
    let net: Vec<f64> = costs.iter().map(|cost| cost.net_return).collect();
    let count = periods as f64;
    let years = count * f64::from(hold_days) / TRADING_DAYS_PER_YEAR;
    let annualize = |equity: f64| (equity.powf(1.0 / years) - 1.0) * 100.0;
    let (equity, drawdown) = compound(returns.iter().copied());
    let (benchmark_equity, benchmark_drawdown) = compound(benchmarks.iter().copied());
    let (net_equity, net_drawdown) = compound(costs.iter().map(|cost| cost.net_return));
    let (etf_equity, etf_drawdown) = compound(etf.iter().copied());
    let period_end_dd = round2(drawdown * 100.0);
    let period_end_net_dd = round2(net_drawdown * 100.0);
    let daily_dd = rows.iter().all(|row| row.daily_complete).then(|| {
        round2(
            compound_daily(
                returns
                    .iter()
                    .copied()
                    .zip(rows.iter().map(|r| r.daily_returns.as_slice())),
            ) * 100.0,
        )
    });
    let daily_net_dd = rows.iter().all(|row| row.daily_complete).then(|| {
        round2(
            compound_daily(
                costs
                    .iter()
                    .map(|cost| cost.net_return)
                    .zip(rows.iter().map(|r| r.daily_net_returns.as_slice())),
            ) * 100.0,
        )
    });
    let invested_turnover: Vec<f64> = rows
        .iter()
        .zip(costs)
        .filter(|(row, _)| row.holdings > 0)
        .map(|(_, cost)| cost.turnover)
        .collect();
    let ratio = |hits: usize| round4(hits as f64 / count);
    BacktestSummary {
        periods: periods as i64,
        invested_periods: invested.len() as i64,
        market_filtered_periods: rows.iter().filter(|row| row.market_filtered).count() as i64,
        stop_exits: rows.iter().map(stop_count).sum(),
        etf_missing_periods: rows.iter().filter(|row| row.etf_return.is_none()).count() as i64,
        avg_holdings: (!invested.is_empty()).then(|| {
            round2(
                invested.iter().map(|row| row.holdings as f64).sum::<f64>() / invested.len() as f64,
            )
        }),
        avg_turnover: (!invested_turnover.is_empty()).then(|| {
            round4(invested_turnover.iter().sum::<f64>() / invested_turnover.len() as f64)
        }),
        total_return: Some(round2((equity - 1.0) * 100.0)),
        total_return_after_costs: Some(round2((net_equity - 1.0) * 100.0)),
        benchmark_total_return: Some(round2((benchmark_equity - 1.0) * 100.0)),
        annualized_return: Some(round2(annualize(equity))),
        annualized_return_after_costs: Some(round2(annualize(net_equity))),
        benchmark_annualized_return: Some(round2(annualize(benchmark_equity))),
        max_drawdown: Some(period_end_dd),
        max_drawdown_after_costs: Some(period_end_net_dd),
        period_end_max_drawdown: Some(period_end_dd),
        period_end_max_drawdown_after_costs: Some(period_end_net_dd),
        daily_max_drawdown: daily_dd,
        daily_max_drawdown_after_costs: daily_net_dd,
        benchmark_max_drawdown: Some(round2(benchmark_drawdown * 100.0)),
        win_rate: Some(ratio(returns.iter().filter(|r| **r > 0.0).count())),
        beat_market_rate: Some(ratio(
            returns
                .iter()
                .zip(&benchmarks)
                .filter(|(r, b)| r > b)
                .count(),
        )),
        avg_period_return: Some(round2(returns.iter().sum::<f64>() / count)),
        avg_benchmark_return: Some(round2(benchmarks.iter().sum::<f64>() / count)),
        etf_total_return: has_etf.then(|| round2((etf_equity - 1.0) * 100.0)),
        etf_annualized_return: has_etf.then(|| round2(annualize(etf_equity))),
        etf_max_drawdown: has_etf.then(|| round2(etf_drawdown * 100.0)),
        beat_etf_rate: has_etf
            .then(|| ratio(returns.iter().zip(&etf).filter(|(r, e)| r > e).count())),
        volatility: risk(&returns, per_year).map(|(volatility, _)| volatility),
        sharpe_ratio: risk(&net, per_year).and_then(|(_, sharpe)| sharpe),
        benchmark_volatility: risk(&benchmarks, per_year).map(|(volatility, _)| volatility),
        benchmark_sharpe_ratio: risk(&benchmarks, per_year).and_then(|(_, sharpe)| sharpe),
        etf_volatility: risk(&etf, per_year)
            .filter(|_| has_etf)
            .map(|(volatility, _)| volatility),
        etf_sharpe_ratio: risk(&etf, per_year)
            .filter(|_| has_etf)
            .and_then(|(_, sharpe)| sharpe),
    }
}

/// 每期報酬（%）的年化波動率（%）與夏普值（無風險利率 0），`per_year` 為每年期數；不足 2 期回
/// `None`，報酬沒有波動時夏普值為 `None`。兩者都四捨五入到小數兩位。
pub(super) fn risk(returns: &[f64], per_year: f64) -> Option<(f64, Option<f64>)> {
    if returns.len() < 2 {
        return None;
    }
    let count = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / count;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (count - 1.0);
    let deviation = variance.sqrt();
    let sharpe = (deviation > 0.0).then(|| round2(mean / deviation * per_year.sqrt()));
    Some((round2(deviation * per_year.sqrt()), sharpe))
}

/// 依換股日的年度彙總，由舊到新；`costs` 與 `rows` 一一對應。
pub(super) fn yearly(rows: &[PeriodRow], costs: &[PeriodCost]) -> Vec<BacktestYear> {
    let paired: Vec<(&PeriodRow, &PeriodCost)> = rows.iter().zip(costs).collect();
    paired
        .chunk_by(|(a, _), (b, _)| a.date.year() == b.date.year())
        .map(year_summary)
        .collect()
}

/// 同一年的各期：各年各自從淨值 1 起算，空手與缺值的報酬算 0。
pub(super) fn year_summary(chunk: &[(&PeriodRow, &PeriodCost)]) -> BacktestYear {
    let count = chunk.len() as f64;
    let returns: Vec<f64> = chunk
        .iter()
        .map(|(row, _)| row.avg_return.unwrap_or(0.0))
        .collect();
    let benchmarks: Vec<f64> = chunk
        .iter()
        .map(|(row, _)| row.benchmark_return.unwrap_or(0.0))
        .collect();
    let has_etf = chunk.iter().any(|(row, _)| row.etf_return.is_some());
    BacktestYear {
        year: chunk[0].0.date.year(),
        periods: chunk.len() as i64,
        total_return: cumulative(returns.iter().copied()),
        total_return_after_costs: cumulative(chunk.iter().map(|(_, cost)| cost.net_return)),
        benchmark_total_return: cumulative(benchmarks.iter().copied()),
        etf_total_return: has_etf
            .then(|| cumulative(chunk.iter().map(|(row, _)| row.etf_return.unwrap_or(0.0)))),
        win_rate: round4(returns.iter().filter(|r| **r > 0.0).count() as f64 / count),
        beat_market_rate: round4(
            returns
                .iter()
                .zip(&benchmarks)
                .filter(|(r, b)| r > b)
                .count() as f64
                / count,
        ),
    }
}

/// 各期報酬（%）連乘後的累積報酬（%），四捨五入到小數兩位。
pub(super) fn cumulative(returns: impl IntoIterator<Item = f64>) -> f64 {
    round2((compound(returns).0 - 1.0) * 100.0)
}
