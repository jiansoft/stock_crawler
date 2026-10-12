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

    for (position, point) in rest.iter().enumerate() {
        let flow = point.buys - point.sells;
        let daily = if previous.market_value > Decimal::ZERO {
            ((point.market_value - flow) / previous.market_value)
                .to_f64()
                .unwrap_or(1.0)
        } else {
            1.0
        };
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

/// 金額四捨五入到小數兩位。
fn money(value: Decimal) -> f64 {
    value.round_dp(2).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).expect("有效日期")
    }

    fn point(date: NaiveDate, value: Decimal, buys: Decimal, sells: Decimal) -> ValuePoint {
        ValuePoint {
            date,
            market_value: value,
            buys,
            sells,
        }
    }

    /// 加碼與賣出不算報酬：市值從 100 到 210，但其中 100 是新投入，實際只賺 10%。
    #[test]
    fn contributions_are_excluded_from_return() {
        let points = [
            point(day(1, 30), dec!(100), dec!(0), dec!(0)),
            point(day(2, 2), dec!(110), dec!(0), dec!(0)),
            point(day(2, 3), dec!(210), dec!(100), dec!(0)),
            point(day(2, 4), dec!(110), dec!(0), dec!(100)),
        ];
        let index = [
            IndexClose {
                date: day(1, 30),
                close: dec!(20000),
            },
            IndexClose {
                date: day(2, 4),
                close: dec!(21000),
            },
        ];
        let performance = evaluate(&points, &index).expect("兩天以上");
        assert_eq!(performance.return_percent, 10.0);
        assert_eq!((performance.buys, performance.sells), (100.0, 100.0));
        assert_eq!(performance.benchmark_return_percent, Some(5.0));
        assert_eq!(performance.annualized_return_percent, None);
        assert_eq!(performance.max_drawdown_percent, 0.0);
        assert_eq!(performance.max_drawdown_peak, None);

        // 1 月只有基準日、沒有報酬；2 月一個月 +10%。
        assert_eq!(performance.months.len(), 1);
        let february = &performance.months[0];
        assert_eq!(february.month, "2026-02");
        assert_eq!(
            (february.return_percent, february.net_contribution),
            (10.0, 0.0)
        );
        assert_eq!(february.end_value, 110.0);
        assert_eq!(february.benchmark_return_percent, Some(5.0));
    }

    /// 最大回撤以時間加權的財富指數計算，並記下高點與低點日期；月報酬逐月複利。
    #[test]
    fn drawdown_and_monthly_returns() {
        let points = [
            point(day(3, 31), dec!(100), dec!(0), dec!(0)),
            point(day(4, 15), dec!(120), dec!(0), dec!(0)),
            point(day(4, 30), dec!(90), dec!(0), dec!(0)),
            point(day(5, 29), dec!(108), dec!(0), dec!(0)),
        ];
        let performance = evaluate(&points, &[]).expect("兩天以上");
        assert_eq!(performance.return_percent, 8.0);
        assert_eq!(performance.max_drawdown_percent, -25.0);
        assert_eq!(performance.max_drawdown_peak, Some(day(4, 15)));
        assert_eq!(performance.max_drawdown_trough, Some(day(4, 30)));
        assert_eq!(performance.benchmark_return_percent, None);
        let returns: Vec<f64> = performance
            .months
            .iter()
            .map(|month| month.return_percent)
            .collect();
        assert_eq!(returns, vec![-10.0, 20.0]);
        assert!(performance.months[0].benchmark_return_percent.is_none());
    }

    /// 前一日市值為 0 時報酬記 0；滿一年才有年化報酬；少於兩天回傳 None。
    #[test]
    fn zero_base_and_annualized() {
        let start = NaiveDate::from_ymd_opt(2025, 1, 2).expect("有效日期");
        let points = [
            point(start, dec!(0), dec!(0), dec!(0)),
            point(day(1, 2), dec!(100), dec!(100), dec!(0)),
            point(day(1, 5), dec!(121), dec!(0), dec!(0)),
        ];
        let performance = evaluate(&points, &[]).expect("兩天以上");
        assert_eq!(performance.return_percent, 21.0);
        // 2025-01-02 到 2026-01-05 共 368 天。
        let annualized = performance.annualized_return_percent.expect("滿一年");
        assert!((20.0..21.0).contains(&annualized), "{annualized}");

        assert!(evaluate(&points[..1], &[]).is_none());
        assert!(evaluate(&[], &[]).is_none());
    }
}
