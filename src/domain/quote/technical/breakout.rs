//! 區間突破：整理一段時間後，當天收盤帶量突破整理區間高點，且離突破價還不遠。
//!
//! 規則移植自使用者 2026-10-29 提供的 XQ 選股腳本「區間整理＋帶量突破＋不追高」（預設參數）：
//!
//! | 項目 | 規則 |
//! |---|---|
//! | 突破價 | 前 [`BASE_DAYS`] 個交易日（不含當天）的最高價 |
//! | 區間底 | 前 [`BASE_DAYS`] 個交易日（不含當天）的最低價 |
//! | 整理幅度 | 突破價 ≤ 區間底 ×（1 + [`MAX_BASE_PERCENT`]%） |
//! | 收盤突破 | 收盤 > 突破價 |
//! | 帶量 | 當天成交量 ≥ 前 [`VOLUME_DAYS`] 個交易日（不含當天）均量 × [`MIN_VOLUME_RATIO`] |
//! | 不追高 | 收盤 ≤ 突破價 ×（1 + [`MAX_CHASE_PERCENT`]%） |
//!
//! 基準都排除當天，否則當天的高點會墊高突破價，收盤永遠突破不了。價格用呼叫端傳入的日線
//! （訊號計算時已還原權值，對應腳本的還原日線）；成交量不還原。「整理」只是底部成形的代理，
//! 不辨識特定型態。
//!
//! ## 假突破
//!
//! 突破後 [`FAILED_WITHIN_DAYS`] 個交易日內，收盤第一次跌回突破價之下記一筆 `failed`
//! （[`failed_breakout_signal`]）：只看最近一次突破，突破後到前一天的收盤都要守在突破價以上（含等於），
//! 所以同一次突破只會記一次。

use chrono::NaiveDate;

use super::RANGE_BREAKOUT_INDICATOR;
use super::bars::PriceBar;
use super::detection::price_text;
use super::indicators::{bar_high, bar_low};
use super::signal::{SignalKind, TechnicalSignal};

/// 整理區間的交易日數（不含當天）。
pub const BASE_DAYS: usize = 30;
/// 整理區間最大幅度（%）：突破價相對區間底的漲幅上限。
pub const MAX_BASE_PERCENT: f64 = 20.0;
/// 基準均量的交易日數（不含當天）。
pub const VOLUME_DAYS: usize = 20;
/// 突破當天至少要有的量比（當天量 ÷ 基準均量）。
pub const MIN_VOLUME_RATIO: f64 = 1.5;
/// 收盤距突破價的最大漲幅（%）。
pub const MAX_CHASE_PERCENT: f64 = 5.0;
/// 判斷需要的日線根數（含當天）。
pub const RANGE_BREAKOUT_BARS: usize = BASE_DAYS + 1;
/// 判斷需要的成交量根數（含當天）。
pub const RANGE_BREAKOUT_VOLUME_BARS: usize = VOLUME_DAYS + 1;
/// 突破後幾個交易日內收盤跌回突破價之下算假突破。
pub const FAILED_WITHIN_DAYS: usize = 10;
/// 判斷假突破需要的日線根數（含當天）。
pub const FAILED_BREAKOUT_BARS: usize = RANGE_BREAKOUT_BARS + FAILED_WITHIN_DAYS;
/// 判斷假突破需要的成交量根數（含當天）。
pub const FAILED_BREAKOUT_VOLUME_BARS: usize = RANGE_BREAKOUT_VOLUME_BARS + FAILED_WITHIN_DAYS;

/// 某交易日的成交量（未還原）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeBar {
    /// 交易日。
    pub date: NaiveDate,
    /// 成交量。
    pub volume: f64,
}

/// 最新一日（`bars` 最後一根）相對整理區間的狀態；`breaks_out` 為四個條件是否同時成立。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeBreakoutState {
    /// 突破價：前 [`BASE_DAYS`] 個交易日（不含當天）的最高價。
    pub pivot: f64,
    /// 區間底：前 [`BASE_DAYS`] 個交易日（不含當天）的最低價。
    pub floor: f64,
    /// 整理幅度（%）：突破價相對區間底的漲幅。
    pub base_width_percent: f64,
    /// 基準均量：前 [`VOLUME_DAYS`] 個交易日（不含當天）的平均成交量。
    pub average_volume: f64,
    /// 量比：當天成交量 ÷ 基準均量。
    pub volume_ratio: f64,
    /// 收盤距突破價（%）；負數代表還在突破價之下。
    pub distance_percent: f64,
    /// 整理幅度、收盤突破、帶量、不追高是否同時成立。
    pub breaks_out: bool,
}

/// 最新一日（`bars` 最後一根）相對整理區間的狀態。
///
/// `volumes` 由舊到新，最後 [`RANGE_BREAKOUT_VOLUME_BARS`] 根的日期必須與 `bars` 最後同樣根數的日期
/// 一一相同，否則（缺資料、沒對齊）回 `None`。日線不足 [`RANGE_BREAKOUT_BARS`] 根、區間底或基準均量
/// 不是正數時也回 `None`。
pub fn range_breakout_state(
    bars: &[PriceBar],
    volumes: &[VolumeBar],
) -> Option<RangeBreakoutState> {
    let base_start = bars.len().checked_sub(RANGE_BREAKOUT_BARS)?;
    let volume_start = volumes.len().checked_sub(RANGE_BREAKOUT_VOLUME_BARS)?;
    let bar_tail = &bars[bars.len() - RANGE_BREAKOUT_VOLUME_BARS..];
    let volumes = &volumes[volume_start..];
    let aligned = bar_tail
        .iter()
        .zip(volumes)
        .all(|(bar, volume)| bar.date == volume.date);
    if !aligned {
        return None;
    }

    let (last, base) = bars[base_start..].split_last()?;
    let pivot = base.iter().map(bar_high).fold(f64::MIN, f64::max);
    let floor = base.iter().map(bar_low).fold(f64::MAX, f64::min);
    let (today_volume, prior_volumes) = volumes.split_last()?;
    let average_volume =
        prior_volumes.iter().map(|bar| bar.volume).sum::<f64>() / prior_volumes.len() as f64;
    if floor <= 0.0 || average_volume <= 0.0 {
        return None;
    }

    let close = last.close;
    let breaks_out = pivot <= floor * (1.0 + MAX_BASE_PERCENT / 100.0)
        && close > pivot
        && today_volume.volume >= average_volume * MIN_VOLUME_RATIO
        && close <= pivot * (1.0 + MAX_CHASE_PERCENT / 100.0);
    Some(RangeBreakoutState {
        pivot,
        floor,
        base_width_percent: (pivot / floor - 1.0) * 100.0,
        average_volume,
        volume_ratio: today_volume.volume / average_volume,
        distance_percent: (close / pivot - 1.0) * 100.0,
        breaks_out,
    })
}

/// 最新一日（`bars` 最後一根）是否出現區間突破；判斷見 [`range_breakout_state`]。
pub fn range_breakout_signal(bars: &[PriceBar], volumes: &[VolumeBar]) -> Option<TechnicalSignal> {
    let state = range_breakout_state(bars, volumes)?;
    if !state.breaks_out {
        return None;
    }
    let close = bars.last()?.close;
    Some(TechnicalSignal {
        indicator: RANGE_BREAKOUT_INDICATOR,
        kind: SignalKind::Breakout,
        description: format!(
            "收盤 {} 帶量突破 {BASE_DAYS} 日整理區間高點 {}（整理幅度 {:.2}%，量比 {:.2} 倍，距突破價 {:.2}%）",
            price_text(close),
            price_text(state.pivot),
            state.base_width_percent,
            state.volume_ratio,
            state.distance_percent,
        ),
    })
}

/// 最新一日（`bars` 最後一根）是否為假突破：前 1～[`FAILED_WITHIN_DAYS`] 個交易日內最近一次區間突破之後，
/// 收盤一直守在突破價以上（含等於），直到當天第一次收在突破價之下。
///
/// `bars` 與 `volumes` 由舊到新、最後一根同一天；往前推的每一天都以同樣根數截斷兩者，交給
/// [`range_breakout_state`] 判斷當天是否突破（成交量沒對齊時視為沒有突破）。
pub fn failed_breakout_signal(bars: &[PriceBar], volumes: &[VolumeBar]) -> Option<TechnicalSignal> {
    let (today, _) = bars.split_last()?;
    for days_ago in 1..=FAILED_WITHIN_DAYS {
        let bar_end = bars.len().checked_sub(days_ago)?;
        let volume_end = volumes.len().checked_sub(days_ago)?;
        let Some(state) = range_breakout_state(&bars[..bar_end], &volumes[..volume_end]) else {
            continue;
        };
        if !state.breaks_out {
            continue;
        }
        let held = bars[bar_end..bars.len() - 1]
            .iter()
            .all(|bar| bar.close >= state.pivot);
        if !held || today.close >= state.pivot {
            return None;
        }
        return Some(TechnicalSignal {
            indicator: RANGE_BREAKOUT_INDICATOR,
            kind: SignalKind::Failed,
            description: format!(
                "收盤 {} 跌回 {} 區間突破的突破價 {} 之下（突破後第 {days_ago} 個交易日）",
                price_text(today.close),
                bars[bar_end - 1].date,
                price_text(state.pivot),
            ),
        });
    }
    None
}

/// 最新一日的區間突破與假突破訊號（各至多一筆）；判斷見 [`range_breakout_signal`] 與
/// [`failed_breakout_signal`]。
pub fn range_breakout_signals(
    bars: &[PriceBar],
    volumes: &[VolumeBar],
) -> impl Iterator<Item = TechnicalSignal> {
    range_breakout_signal(bars, volumes)
        .into_iter()
        .chain(failed_breakout_signal(bars, volumes))
}

#[cfg(test)]
mod tests;
