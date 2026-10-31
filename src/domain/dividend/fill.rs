//! 填息（填權）分析：除權息後股價回到除權息前一日收盤價花了幾個交易日。
//!
//! 純計算、不做 I/O。規則：
//!
//! - **除權息前一日收盤**：除權息日之前最後一筆收盤價（大於 0）。
//! - **填息目標價**：除權息前一日收盤依配股調整（[`fill_target`](crate::domain::dividend::fill::fill_target)）；沒有股票股利時就是前一日收盤。
//! - **填息**：除權息日起第一個收盤價大於等於填息目標價的交易日；除權息當日就回到算第 1 天。
//! - 收盤價為 0（停牌、無成交）的日子不算交易日。

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

/// 股票面額（元）：股票股利每 10 元配 1 股。
const PAR_VALUE: Decimal = Decimal::TEN;

/// 往除權息日前多抓幾天日線，確保長假或停牌後仍找得到除權息前一日的收盤。
pub const PRE_EX_LOOKBACK_DAYS: u64 = 30;

/// 一次除權息：同一天的現金股利與股票股利合併成一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExRightEvent {
    pub ex_date: NaiveDate,
    /// 現金股利（元）；沒有為 0。
    pub cash_dividend: Decimal,
    /// 股票股利（元）；沒有為 0。
    pub stock_dividend: Decimal,
}

/// 單日收盤價。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyClose {
    pub date: NaiveDate,
    pub close: Decimal,
}

/// 單次除權息的填息結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillOutcome {
    pub event: ExRightEvent,
    /// 除權息前一個交易日。
    pub pre_ex_date: NaiveDate,
    /// 除權息前一個交易日的收盤價（未依配股調整）。
    pub pre_ex_close: Decimal,
    /// 填息目標價：`pre_ex_close` 依配股調整，見 [`fill_target()`]。
    pub fill_target: Decimal,
    /// 填息日；尚未填息為 `None`。
    pub fill_date: Option<NaiveDate>,
    /// 除權息日起算第幾個交易日填息（除權息當日為 1）。
    pub trading_days_to_fill: Option<u32>,
    /// 除權息日到最新一筆收盤共幾個交易日（含除權息日）。
    pub trading_days_since_ex: u32,
    /// 最新一筆收盤價與日期。
    pub latest_close: Decimal,
    pub latest_date: NaiveDate,
}

impl FillOutcome {
    /// 尚未填息時，最新收盤距離填息價還差多少（%，正數表示仍低於填息價）；已填息為 `None`。
    pub fn gap_percent(&self) -> Option<Decimal> {
        if self.fill_date.is_some() || self.fill_target.is_zero() {
            return None;
        }
        Some(
            ((self.fill_target - self.latest_close) * Decimal::ONE_HUNDRED / self.fill_target)
                .round_dp(2),
        )
    }
}

/// 依配股調整的填息目標價：`除權息前收盤 ÷ (1 + 股票股利 ÷ 面額 10 元)`，取到小數第 2 位。
///
/// 配股讓股數變成 `1 + 股票股利 ÷ 10` 倍，股價回到這個價位時，股票市值已回到除權前；
/// 現金股利那部分仍要漲回來才算填息。直接比除權前收盤時，配股 2 元的股票要漲 20% 才回得去，
/// 實際上持股人的資產早已回到除權前。沒有股票股利（或資料為負）時就是除權息前收盤。
#[must_use]
pub fn fill_target(pre_ex_close: Decimal, stock_dividend: Decimal) -> Decimal {
    if stock_dividend <= Decimal::ZERO {
        return pre_ex_close;
    }
    let share_ratio = Decimal::ONE + stock_dividend / PAR_VALUE;
    (pre_ex_close / share_ratio).round_dp(2)
}

/// 計算單次除權息的填息結果。
///
/// `closes` 依日期升冪。找不到除權息前的收盤，或除權息日之後還沒有任何收盤時回 `None`。
pub fn fill_outcome(event: ExRightEvent, closes: &[DailyClose]) -> Option<FillOutcome> {
    let traded = |c: &&DailyClose| c.close > Decimal::ZERO;
    let pre = closes
        .iter()
        .filter(traded)
        .rfind(|c| c.date < event.ex_date)?;
    let after: Vec<&DailyClose> = closes
        .iter()
        .filter(traded)
        .filter(|c| c.date >= event.ex_date)
        .collect();
    let latest = *after.last()?;
    let target = fill_target(pre.close, event.stock_dividend);
    let filled = after.iter().position(|c| c.close >= target).map(|index| {
        (
            after[index].date,
            u32::try_from(index + 1).unwrap_or(u32::MAX),
        )
    });

    Some(FillOutcome {
        event,
        pre_ex_date: pre.date,
        pre_ex_close: pre.close,
        fill_target: target,
        fill_date: filled.map(|(date, _)| date),
        trading_days_to_fill: filled.map(|(_, days)| days),
        trading_days_since_ex: u32::try_from(after.len()).unwrap_or(u32::MAX),
        latest_close: latest.close,
        latest_date: latest.date,
    })
}

/// 多次除權息的填息統計。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillSummary {
    /// 統計的除權息次數。
    pub events: usize,
    /// 已填息的次數。
    pub filled: usize,
    /// 已填息者的平均填息交易日數（取到小數第 1 位）。
    pub average_trading_days: Option<f64>,
    /// 已填息者的填息交易日數中位數。
    pub median_trading_days: Option<f64>,
}

/// 統計已填息次數、平均與中位數填息天數。
pub fn summarize(outcomes: &[FillOutcome]) -> FillSummary {
    let mut days: Vec<u32> = outcomes
        .iter()
        .filter_map(|o| o.trading_days_to_fill)
        .collect();
    days.sort_unstable();
    let average_trading_days = (!days.is_empty()).then(|| {
        let total: f64 = days.iter().map(|&d| f64::from(d)).sum();
        (total / days.len() as f64 * 10.0).round() / 10.0
    });
    let median_trading_days = match days.len() {
        0 => None,
        len if len % 2 == 1 => Some(f64::from(days[len / 2])),
        len => Some((f64::from(days[len / 2 - 1]) + f64::from(days[len / 2])) / 2.0),
    };
    FillSummary {
        events: outcomes.len(),
        filled: days.len(),
        average_trading_days,
        median_trading_days,
    }
}

/// 填息分析的資料來源：除權息事件、日收盤與最新收盤日。
#[async_trait]
pub trait DividendFillQuery: Send + Sync {
    /// 股票最新一筆日線的日期；沒有日線為 `None`。
    async fn fetch_latest_quote_date(&self, symbol: &str) -> Result<Option<NaiveDate>>;

    /// 股票的每次除權息（同一天的現金與股票股利合併），由新到舊；除權息日格式錯誤的略過。
    async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>>;

    /// `since`（含）起的日收盤，依日期升冪。
    async fn fetch_daily_closes(&self, symbol: &str, since: NaiveDate) -> Result<Vec<DailyClose>>;
}

/// 除權息日在 `since`（不含）～`until`（含）之間的每次除權息的填息結果，依除權息日由舊到新；
/// 期間內沒有除權息時不查日線。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub async fn recent_outcomes(
    query: &impl DividendFillQuery,
    symbol: &str,
    since: NaiveDate,
    until: NaiveDate,
) -> Result<Vec<FillOutcome>> {
    let mut events: Vec<ExRightEvent> = query
        .fetch_ex_right_events(symbol)
        .await?
        .into_iter()
        .filter(|event| event.ex_date > since && event.ex_date <= until)
        .collect();
    events.sort_by_key(|event| event.ex_date);
    let Some(oldest) = events.first() else {
        return Ok(Vec::new());
    };
    let closes = query
        .fetch_daily_closes(symbol, oldest.ex_date - Days::new(PRE_EX_LOOKBACK_DAYS))
        .await?;
    Ok(events
        .into_iter()
        .filter_map(|event| fill_outcome(event, &closes))
        .collect())
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, d).expect("日期應合法")
    }

    fn closes(prices: &[(u32, Decimal)]) -> Vec<DailyClose> {
        prices
            .iter()
            .map(|&(d, close)| DailyClose {
                date: day(d),
                close,
            })
            .collect()
    }

    fn event(ex_day: u32) -> ExRightEvent {
        ExRightEvent {
            ex_date: day(ex_day),
            cash_dividend: dec!(5),
            stock_dividend: Decimal::ZERO,
        }
    }

    /// 第 3 個交易日收盤回到除息前一日收盤即填息；停牌（收盤 0）的日子不算交易日。
    #[test]
    fn fill_outcome_counts_trading_days_from_the_ex_date() {
        let prices = closes(&[
            (14, dec!(98)),
            (15, dec!(100)),
            (16, dec!(95)),
            (17, Decimal::ZERO),
            (18, dec!(99)),
            (21, dec!(100)),
            (22, dec!(97)),
        ]);
        let outcome = fill_outcome(event(16), &prices).expect("應算得出結果");
        assert_eq!(
            (outcome.pre_ex_date, outcome.pre_ex_close),
            (day(15), dec!(100))
        );
        assert_eq!(outcome.fill_date, Some(day(21)));
        assert_eq!(outcome.trading_days_to_fill, Some(3));
        assert_eq!(outcome.trading_days_since_ex, 4);
        assert_eq!(
            (outcome.latest_date, outcome.latest_close),
            (day(22), dec!(97))
        );
        assert_eq!(outcome.gap_percent(), None, "已填息不算差距");
    }

    /// 除息當日就回到前一日收盤算第 1 天；還沒填息時算出距離填息價的百分比。
    #[test]
    fn fill_outcome_handles_same_day_fill_and_unfilled_gap() {
        let same_day = closes(&[(15, dec!(100)), (16, dec!(100.5))]);
        let outcome = fill_outcome(event(16), &same_day).expect("應算得出結果");
        assert_eq!(outcome.trading_days_to_fill, Some(1));

        let unfilled = closes(&[(15, dec!(80)), (16, dec!(75)), (17, dec!(76))]);
        let outcome = fill_outcome(event(16), &unfilled).expect("應算得出結果");
        assert_eq!(outcome.fill_date, None);
        assert_eq!(outcome.trading_days_since_ex, 2);
        assert_eq!(outcome.gap_percent(), Some(dec!(5)));
    }

    /// 沒有除權息前的收盤，或除權息後還沒有收盤時算不出結果。
    #[test]
    fn fill_outcome_needs_closes_on_both_sides() {
        assert_eq!(fill_outcome(event(16), &closes(&[(16, dec!(90))])), None);
        assert_eq!(fill_outcome(event(16), &closes(&[(15, dec!(90))])), None);
        assert_eq!(
            fill_outcome(event(16), &closes(&[(15, Decimal::ZERO), (16, dec!(90))])),
            None,
            "除權息前只有停牌日"
        );
    }

    fn stock_event(ex_day: u32, cash: Decimal, stock: Decimal) -> ExRightEvent {
        ExRightEvent {
            ex_date: day(ex_day),
            cash_dividend: cash,
            stock_dividend: stock,
        }
    }

    /// 目標價只扣配股稀釋：純現金不變、配股 2 元除以 1.2、負值視為沒有配股。
    #[test]
    fn fill_target_adjusts_for_stock_dividend_only() {
        assert_eq!(fill_target(dec!(100), Decimal::ZERO), dec!(100));
        assert_eq!(fill_target(dec!(60), dec!(2)), dec!(50));
        assert_eq!(fill_target(dec!(100), dec!(0.5)), dec!(95.24));
        assert_eq!(fill_target(dec!(100), dec!(-1)), dec!(100));
    }

    /// 純配股：回到 100 ÷ 1.2 就填權，差距也以調整後目標價計算。
    #[test]
    fn fill_outcome_uses_adjusted_target_for_stock_dividend() {
        let prices = closes(&[
            (15, dec!(60)),
            (16, dec!(48)),
            (17, dec!(49)),
            (18, dec!(50)),
        ]);
        let outcome =
            fill_outcome(stock_event(16, Decimal::ZERO, dec!(2)), &prices).expect("應算得出結果");
        assert_eq!(
            (outcome.pre_ex_close, outcome.fill_target),
            (dec!(60), dec!(50))
        );
        assert_eq!(outcome.trading_days_to_fill, Some(3));

        let unfilled = &prices[..3];
        let outcome =
            fill_outcome(stock_event(16, Decimal::ZERO, dec!(2)), unfilled).expect("應算得出結果");
        assert_eq!(outcome.fill_date, None);
        assert_eq!(outcome.gap_percent(), Some(dec!(2)), "(50 − 49) ÷ 50");
    }

    /// 現金＋配股：目標價扣配股稀釋但不扣現金；配股後當日就站上目標價算第 1 天。
    #[test]
    fn fill_outcome_with_cash_and_stock_dividend() {
        let prices = closes(&[(15, dec!(110)), (16, dec!(97)), (17, dec!(100))]);
        let event = stock_event(16, dec!(2), dec!(1));
        let outcome = fill_outcome(event, &prices).expect("應算得出結果");
        assert_eq!(outcome.fill_target, dec!(100));
        assert_eq!(
            outcome.trading_days_to_fill,
            Some(2),
            "97 未到 100，第 2 天 100 填息"
        );

        let same_day = closes(&[(15, dec!(110)), (16, dec!(101))]);
        let outcome = fill_outcome(event, &same_day).expect("應算得出結果");
        assert_eq!(outcome.trading_days_to_fill, Some(1));
    }

    /// 平均與中位數只看已填息者；偶數筆取中間兩筆平均；沒有已填息時為 None。
    #[test]
    fn summarize_averages_filled_events_only() {
        let outcome = |days: Option<u32>| FillOutcome {
            event: event(16),
            pre_ex_date: day(15),
            pre_ex_close: dec!(100),
            fill_target: dec!(100),
            fill_date: days.map(|_| day(20)),
            trading_days_to_fill: days,
            trading_days_since_ex: 10,
            latest_close: dec!(100),
            latest_date: day(30),
        };
        let summary = summarize(&[
            outcome(Some(1)),
            outcome(Some(10)),
            outcome(None),
            outcome(Some(2)),
            outcome(Some(4)),
        ]);
        assert_eq!(
            summary,
            FillSummary {
                events: 5,
                filled: 4,
                average_trading_days: Some(4.3),
                median_trading_days: Some(3.0),
            }
        );
        let odd = summarize(&[outcome(Some(3)), outcome(Some(1)), outcome(Some(8))]);
        assert_eq!(odd.median_trading_days, Some(3.0));
        let none = summarize(&[outcome(None)]);
        assert_eq!(
            (
                none.filled,
                none.average_trading_days,
                none.median_trading_days
            ),
            (0, None, None)
        );
    }
}
