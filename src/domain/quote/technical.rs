//! # 技術指標
//!
//! 由日線（收盤、最高、最低）計算常用技術指標，並在最新一日找出交叉與超買超賣訊號。
//!
//! - 均線（MA）：最近 N 個交易日收盤價的簡單平均；乖離率＝(收盤 − MA) ÷ MA × 100。
//! - RSI：Wilder 平滑法，首值為前 N 日漲跌的簡單平均，之後
//!   `平均 = (前平均 × (N − 1) + 當日) ÷ N`；平均跌幅為 0 時 RSI 為 100（全平盤為 50）。
//! - KD(9,3,3)：RSV＝(收盤 − 9 日最低) ÷ (9 日最高 − 9 日最低) × 100，9 日高低相同時取 50；
//!   K＝⅔ 前 K＋⅓ RSV、D＝⅔ 前 D＋⅓ K，起始 K、D 為 50。
//! - MACD(12,26,9)：DIF＝收盤價 EMA12 − EMA26（EMA 以第一日收盤為起點），
//!   訊號線＝DIF 的 9 日 EMA（以第一個 DIF 為起點），柱狀體＝DIF − 訊號線。
//!
//! 計算前先以 [`adjust_bars`] 把日線還原權值（向後還原：最新一日為實際價格，除權息、
//! 減資、分割之前的價格乘上還原因子），避免除權息跳空被誤判成跌破均線或 52 週新低。
//! 樣本不足的指標為 `None`。KD、RSI 與 EMA 都帶有遞迴記憶，呼叫端應多取 [`WARMUP_BARS`]
//! 日讓起始值的影響衰減到可忽略。
//!
//! 子模組：[`bars`]（日線與還原權值）、[`indicators`]（指標計算）、[`signal`]（訊號型別與存取介面）、
//! [`detection`]（訊號判斷）。

/// 均線週期：週、雙週、月、季、半年、年線。
pub const MA_PERIODS: [usize; 6] = [5, 10, 20, 60, 120, 240];

/// 計算前額外讀取的交易日數：涵蓋年線所需的 240 日，並讓 EMA、KD 的起始值影響衰減。
pub const WARMUP_BARS: usize = 360;

/// 52 週新高／新低訊號的來源名稱。
pub const FIFTY_TWO_WEEK_INDICATOR: &str = "52_week";

/// 相對強度分數穿越 80 分的訊號來源名稱（收盤後寫入每日相對強度時產生，見
/// [`super::relative_strength::crossing_signals`]）。
pub const RS80_INDICATOR: &str = "rs80";

/// 相對強度分數穿越 50 分的訊號來源名稱。
pub const RS50_INDICATOR: &str = "rs50";

/// 區間整理後帶量突破的訊號來源名稱（見 [`breakout`]）。
pub const RANGE_BREAKOUT_INDICATOR: &str = "range_breakout";

mod bars;
pub mod breakout;
mod detection;
mod indicators;
mod signal;

pub use self::bars::{
    PriceAdjustment, PriceBar, PriceBarRepository, adjust_bars, adjustment_factors,
};
pub use self::breakout::{
    RangeBreakoutState, VolumeBar, range_breakout_signal, range_breakout_state,
};
pub use self::detection::{fifty_two_week_signals, signals};
pub use self::indicators::{IndicatorPoint, compute};
pub use self::signal::{
    OUTCOME_HORIZON, RankedSignal, RankedSignalQuery, SIGNAL_INDICATORS, SIGNAL_KINDS, SignalKind,
    SignalOutcome, StockTechnicalSignal, TechnicalSignal, TechnicalSignalQuery,
    TechnicalSignalRepository,
};

#[cfg(test)]
mod tests;
