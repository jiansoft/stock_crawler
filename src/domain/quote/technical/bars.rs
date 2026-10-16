//! 日線與還原權值：除權息、減資、分割造成的跳空，以向後還原因子調整歷史價格。

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 單日日線。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceBar {
    /// 交易日。
    pub date: NaiveDate,
    /// 收盤價。
    pub close: f64,
    /// 最高價；未提供（0）時以收盤價代替。
    pub high: f64,
    /// 最低價；未提供（0）時以收盤價代替。
    pub low: f64,
}

/// 讓報價跳空的除權息或股數變動事件，用來還原歷史日線。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceAdjustment {
    /// 事件日：除權息交易日，或減資、分割後恢復交易的第一個交易日。
    pub date: NaiveDate,
    /// 每股現金股利（元）；沒有為 0。
    pub cash_dividend: f64,
    /// 每股股票股利（元，面額 10 元）；沒有為 0。
    pub stock_dividend: f64,
    /// 減資、分割、反向分割後持有 1 股變成幾股；沒有為 1。
    pub share_ratio: f64,
}

impl PriceAdjustment {
    /// 事件日之前的價格要乘上的還原因子：事件日參考價 ÷ 前一交易日收盤。
    ///
    /// 參考價＝(前收 − 現金股利) ÷ (1 + 股票股利 ÷ 10) ÷ 股數比例。前收不為正、
    /// 參考價不為正或因子為 1 時回 `None`（不調整）。
    pub(super) fn factor(&self, prev_close: f64) -> Option<f64> {
        if prev_close <= 0.0 || self.share_ratio <= 0.0 {
            return None;
        }
        let reference = (prev_close - self.cash_dividend)
            / (1.0 + self.stock_dividend / 10.0)
            / self.share_ratio;
        let factor = reference / prev_close;
        (factor.is_finite() && factor > 0.0 && (factor - 1.0).abs() > f64::EPSILON)
            .then_some(factor)
    }
}

/// 技術指標的日線來源。
#[async_trait]
pub trait PriceBarRepository: Send + Sync {
    /// `until`（含，`None` 為不限）之前最近 `limit` 個有成交的交易日，依日期由舊到新。
    /// 價格為原始成交價，計算前以 [`adjust_bars`] 還原。
    async fn fetch_price_bars(
        &self,
        stock_symbol: &str,
        until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<PriceBar>>;

    /// 多檔股票的除權息與股數變動事件，依代號分組；沒有事件的股票不在結果中。
    async fn fetch_price_adjustments(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, Vec<PriceAdjustment>>>;
}

/// 向後還原日線：最後一根維持實際價格，每個事件日之前的收盤、最高、最低乘上該事件的
/// 還原因子（多個事件累乘），因子見 [`adjustment_factors`]。
pub fn adjust_bars(bars: &[PriceBar], adjustments: &[PriceAdjustment]) -> Vec<PriceBar> {
    bars.iter()
        .zip(adjustment_factors(bars, adjustments))
        .map(|(bar, factor)| PriceBar {
            date: bar.date,
            close: bar.close * factor,
            high: bar.high * factor,
            low: bar.low * factor,
        })
        .collect()
}

/// 每根日線的累計還原因子，與 `bars`（由舊到新）一一對應；最後一根與其後沒有事件的為 1。
///
/// 只採用日期落在第一根之後、最後一根（含）之前的事件，因此以歷史日期為截止日計算時，
/// 截止日之後的除權息不會影響當時的指標。事件日當天沒有成交時，以事件日前最後一根的
/// 收盤計算因子，套用到該根（含）之前。
pub fn adjustment_factors(bars: &[PriceBar], adjustments: &[PriceAdjustment]) -> Vec<f64> {
    let mut factors = vec![1.0; bars.len()];
    let Some(last) = bars.last() else {
        return factors;
    };
    for adjustment in adjustments.iter().filter(|a| a.date <= last.date) {
        // 事件日（含）之後的第一根；之前的都要調整。
        let split = bars.partition_point(|bar| bar.date < adjustment.date);
        if split == 0 {
            continue;
        }
        let Some(factor) = adjustment.factor(bars[split - 1].close) else {
            continue;
        };
        for value in &mut factors[..split] {
            *value *= factor;
        }
    }
    factors
}
