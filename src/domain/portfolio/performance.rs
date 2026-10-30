//! # 持股績效
//!
//! 由每日收盤市值與當天的買進／賣出金額，計算一段期間的時間加權報酬（TWR）、
//! 月報酬與最大回撤，並與同期大盤（加權指數）比較。
//!
//! - 單日報酬＝（當日市值 − 當日淨投入）÷ 前一日市值 − 1；淨投入＝買進成本 − 賣出所得。
//!   扣掉淨投入後，加碼或賣出不會被算成賺賠，只反映股價變化。
//! - 報酬只看股價，**不含現金股利**（股利以現金入帳，不在市值裡）。
//! - 前一日市值為 0（尚未持有）的日子報酬記為 0。
//! - 期間第一天只當基準，不計報酬；落在第一天（含）以前的買賣已反映在起始市值。
//! - 風險指標（beta、alpha、相關係數、追蹤誤差、波動率）以「持股與大盤都有報酬」的交易日計算：
//!   前一日市值為 0 的日子，以及當天或前一個交易日缺指數的日子都不算；不足
//!   [`MIN_RISK_DAYS`] 天時為 `None`。

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use rust_decimal::{Decimal, prelude::ToPrimitive};

/// 單一交易日的市值與淨投入。
#[derive(Debug, Clone, PartialEq)]
pub struct ValuePoint {
    /// 交易日。
    pub date: NaiveDate,
    /// 當日收盤市值。
    pub market_value: Decimal,
    /// 當日買進成本合計（正數）。
    pub buys: Decimal,
    /// 當日賣出所得合計（正數）。
    pub sells: Decimal,
}

/// 大盤指數收盤點位。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndexClose {
    /// 交易日。
    pub date: NaiveDate,
    /// 收盤指數。
    pub close: Decimal,
}

/// 計算風險指標至少需要的成對交易日數。
pub const MIN_RISK_DAYS: usize = 20;

/// 每年約有幾個交易日，用來把日報酬的平均與標準差年化。
pub const TRADING_DAYS_PER_YEAR: f64 = 245.0;

/// 持股相對大盤（加權指數）的風險指標，以每日報酬計算、無風險利率以 0 計。
#[derive(Debug, Clone, PartialEq)]
pub struct RiskStats {
    /// 參與計算的交易日數。
    pub days: usize,
    /// Beta：持股日報酬對大盤日報酬的迴歸斜率（共變異數 ÷ 大盤變異數），四捨五入到小數兩位；
    /// 大盤沒有波動時為 `None`。
    pub beta: Option<f64>,
    /// 年化 alpha（%）：（持股日均報酬 − beta × 大盤日均報酬）× 每年交易日數；beta 為 `None` 時也是 `None`。
    pub alpha_percent: Option<f64>,
    /// 持股與大盤日報酬的相關係數（−1～1），四捨五入到小數兩位；任一方沒有波動時為 `None`。
    pub correlation: Option<f64>,
    /// 年化追蹤誤差（%）：每日（持股 − 大盤）報酬的標準差 × √每年交易日數。
    pub tracking_error_percent: f64,
    /// 持股年化波動率（%）。
    pub volatility_percent: f64,
    /// 大盤年化波動率（%）。
    pub benchmark_volatility_percent: f64,
}

/// 由成對的每日報酬（比率，持股, 大盤）計算風險指標；不足 [`MIN_RISK_DAYS`] 天回傳 `None`。
pub fn risk_stats(pairs: &[(f64, f64)]) -> Option<RiskStats> {
    if pairs.len() < MIN_RISK_DAYS {
        return None;
    }
    let count = pairs.len() as f64;
    let mean = |values: &mut dyn Iterator<Item = f64>| values.sum::<f64>() / count;
    let mean_p = mean(&mut pairs.iter().map(|pair| pair.0));
    let mean_m = mean(&mut pairs.iter().map(|pair| pair.1));
    // 樣本變異數／共變異數（除以 n − 1）。
    let sample = |values: &mut dyn Iterator<Item = f64>| values.sum::<f64>() / (count - 1.0);
    let var_p = sample(&mut pairs.iter().map(|pair| (pair.0 - mean_p).powi(2)));
    let var_m = sample(&mut pairs.iter().map(|pair| (pair.1 - mean_m).powi(2)));
    let cov = sample(
        &mut pairs
            .iter()
            .map(|pair| (pair.0 - mean_p) * (pair.1 - mean_m)),
    );
    let mean_d = mean_p - mean_m;
    let var_d = sample(&mut pairs.iter().map(|pair| (pair.0 - pair.1 - mean_d).powi(2)));
    let annual_sd = |variance: f64| percent(variance.sqrt() * TRADING_DAYS_PER_YEAR.sqrt());
    let beta = (var_m > 0.0).then(|| cov / var_m);
    Some(RiskStats {
        days: pairs.len(),
        beta: beta.map(round2),
        alpha_percent: beta.map(|beta| percent((mean_p - beta * mean_m) * TRADING_DAYS_PER_YEAR)),
        correlation: (var_p > 0.0 && var_m > 0.0).then(|| round2(cov / (var_p * var_m).sqrt())),
        tracking_error_percent: annual_sd(var_d),
        volatility_percent: annual_sd(var_p),
        benchmark_volatility_percent: annual_sd(var_m),
    })
}

/// 單月績效。
#[derive(Debug, Clone, PartialEq)]
pub struct MonthReturn {
    /// 月份，格式 `YYYY-MM`。
    pub month: String,
    /// 月底（期間內該月最後一個交易日）市值。
    pub end_value: f64,
    /// 當月淨投入（買進 − 賣出）。
    pub net_contribution: f64,
    /// 當月時間加權報酬（%）。
    pub return_percent: f64,
    /// 同月大盤報酬（%）；缺指數時為 `None`。
    pub benchmark_return_percent: Option<f64>,
}

/// 期間績效。
#[derive(Debug, Clone, PartialEq)]
pub struct Performance {
    /// 期間第一個交易日（基準日）。
    pub from: NaiveDate,
    /// 期間最後一個交易日。
    pub to: NaiveDate,
    /// 基準日市值。
    pub start_value: f64,
    /// 最後一日市值。
    pub end_value: f64,
    /// 期間買進成本合計。
    pub buys: f64,
    /// 期間賣出所得合計。
    pub sells: f64,
    /// 時間加權報酬（%）。
    pub return_percent: f64,
    /// 年化報酬（%）；期間未滿一年時為 `None`。
    pub annualized_return_percent: Option<f64>,
    /// 最大回撤（%，負數或 0）。
    pub max_drawdown_percent: f64,
    /// 最大回撤的高點日；沒有回撤時為 `None`。
    pub max_drawdown_peak: Option<NaiveDate>,
    /// 最大回撤的低點日；沒有回撤時為 `None`。
    pub max_drawdown_trough: Option<NaiveDate>,
    /// 同期大盤報酬（%）；缺指數時為 `None`。
    pub benchmark_return_percent: Option<f64>,
    /// 各月績效，依月份由舊到新。
    pub months: Vec<MonthReturn>,
    /// 相對大盤的風險指標；成對交易日不足時為 `None`。
    pub risk: Option<RiskStats>,
}

/// 計算期間績效；`points` 需依日期由舊到新，少於兩個交易日時回傳 `None`。
pub fn evaluate(points: &[ValuePoint], index: &[IndexClose]) -> Option<Performance> {
    let (first, rest) = points.split_first()?;
    let last = rest.last()?;

    let mut wealth = 1.0_f64;
    let mut peak = (1.0_f64, first.date);
    let mut drawdown = (0.0_f64, None::<(NaiveDate, NaiveDate)>);
    let mut months: Vec<MonthReturn> = Vec::new();
    let mut month_growth = 1.0_f64;
    let mut month_flow = Decimal::ZERO;
    let mut previous = first;
    let closes: HashMap<NaiveDate, Decimal> = index
        .iter()
        .map(|point| (point.date, point.close))
        .collect();
    let mut pairs: Vec<(f64, f64)> = Vec::new();

    for (position, point) in rest.iter().enumerate() {
        let flow = point.buys - point.sells;
        let daily = if previous.market_value > Decimal::ZERO {
            ((point.market_value - flow) / previous.market_value)
                .to_f64()
                .unwrap_or(1.0)
        } else {
            1.0
        };
        if previous.market_value > Decimal::ZERO
            && let (Some(&before), Some(&close)) =
                (closes.get(&previous.date), closes.get(&point.date))
            && before > Decimal::ZERO
        {
            let market = (close / before).to_f64().unwrap_or(1.0) - 1.0;
            pairs.push((daily - 1.0, market));
        }
        wealth *= daily;
        month_growth *= daily;
        month_flow += flow;

        if wealth > peak.0 {
            peak = (wealth, point.date);
        } else {
            let fall = wealth / peak.0 - 1.0;
            if fall < drawdown.0 {
                drawdown = (fall, Some((peak.1, point.date)));
            }
        }

        let month_ends = rest.get(position + 1).is_none_or(|next| {
            (next.date.year(), next.date.month()) != (point.date.year(), point.date.month())
        });
        if month_ends {
            months.push(MonthReturn {
                month: format!("{:04}-{:02}", point.date.year(), point.date.month()),
                end_value: money(point.market_value),
                net_contribution: money(month_flow),
                return_percent: percent(month_growth - 1.0),
                benchmark_return_percent: None,
            });
            month_growth = 1.0;
            month_flow = Decimal::ZERO;
        }
        previous = point;
    }

    // 各月大盤報酬：月底收盤 ÷ 上個月底（第一個月用基準日）收盤。
    let mut month_start = first.date;
    for month in &mut months {
        let month_end = month_end_date(&month.month, rest).unwrap_or(last.date);
        month.benchmark_return_percent = index_return(index, month_start, month_end);
        month_start = month_end;
    }

    let days = (last.date - first.date).num_days();
    let annualized_return_percent =
        (days >= 365).then(|| percent(wealth.powf(365.0 / days as f64) - 1.0));

    Some(Performance {
        from: first.date,
        to: last.date,
        start_value: money(first.market_value),
        end_value: money(last.market_value),
        buys: money(rest.iter().map(|point| point.buys).sum()),
        sells: money(rest.iter().map(|point| point.sells).sum()),
        return_percent: percent(wealth - 1.0),
        annualized_return_percent,
        max_drawdown_percent: percent(drawdown.0),
        max_drawdown_peak: drawdown.1.map(|(peak, _)| peak),
        max_drawdown_trough: drawdown.1.map(|(_, trough)| trough),
        benchmark_return_percent: index_return(index, first.date, last.date),
        months,
        risk: risk_stats(&pairs),
    })
}

/// 期間內 `YYYY-MM` 月份的最後一個交易日。
fn month_end_date(month: &str, rest: &[ValuePoint]) -> Option<NaiveDate> {
    rest.iter()
        .rev()
        .find(|point| format!("{:04}-{:02}", point.date.year(), point.date.month()) == month)
        .map(|point| point.date)
}

/// 大盤在 `from`～`to` 的報酬（%）：各取當天或之前最近一筆收盤。
fn index_return(index: &[IndexClose], from: NaiveDate, to: NaiveDate) -> Option<f64> {
    let close_on = |date: NaiveDate| {
        index
            .iter()
            .filter(|point| point.date <= date)
            .max_by_key(|point| point.date)
            .map(|point| point.close)
    };
    let (start, end) = (close_on(from)?, close_on(to)?);
    (start > Decimal::ZERO).then(|| percent((end / start).to_f64().unwrap_or(1.0) - 1.0))
}

/// 比率轉百分比並四捨五入到小數兩位。
fn percent(ratio: f64) -> f64 {
    (ratio * 10_000.0).round() / 100.0
}

/// 四捨五入到小數兩位。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// 金額四捨五入到小數兩位。
fn money(value: Decimal) -> f64 {
    value.round_dp(2).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests;
