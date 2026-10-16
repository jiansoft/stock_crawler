//! 技術指標計算：均線、RSI、KD 與 MACD（演算法見 [`super`] 模組說明）。

use chrono::NaiveDate;

use super::MA_PERIODS;
use super::bars::PriceBar;

const KD_PERIOD: usize = 9;
const MACD_FAST: usize = 12;
const MACD_SLOW: usize = 26;
const MACD_SIGNAL: usize = 9;

/// 單日的各項技術指標；樣本不足的欄位為 `None`。
#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorPoint {
    /// 交易日。
    pub date: NaiveDate,
    /// 收盤價。
    pub close: f64,
    /// 依 [`MA_PERIODS`] 順序的均線。
    pub moving_averages: [Option<f64>; MA_PERIODS.len()],
    /// RSI(6)。
    pub rsi6: Option<f64>,
    /// RSI(12)。
    pub rsi12: Option<f64>,
    /// RSI(14)。
    pub rsi14: Option<f64>,
    /// K 值。
    pub k: Option<f64>,
    /// D 值。
    pub d: Option<f64>,
    /// MACD 的 DIF（快慢 EMA 差）。
    pub macd_dif: Option<f64>,
    /// MACD 訊號線（DIF 的 9 日 EMA）。
    pub macd_signal: Option<f64>,
    /// MACD 柱狀體（DIF − 訊號線）。
    pub macd_histogram: Option<f64>,
}

impl IndicatorPoint {
    /// 指定週期的均線；週期不在 [`MA_PERIODS`] 時為 `None`。
    pub fn moving_average(&self, period: usize) -> Option<f64> {
        let index = MA_PERIODS.iter().position(|&p| p == period)?;
        self.moving_averages[index]
    }

    /// 收盤價相對指定均線的乖離率（%）。
    pub fn bias(&self, period: usize) -> Option<f64> {
        let ma = self.moving_average(period).filter(|ma| *ma > 0.0)?;
        Some((self.close - ma) / ma * 100.0)
    }
}

/// 依日線（由舊到新）逐日計算技術指標，回傳與輸入等長、同順序的結果。
pub fn compute(bars: &[PriceBar]) -> Vec<IndicatorPoint> {
    let closes: Vec<f64> = bars.iter().map(|bar| bar.close).collect();
    let mut prefix = Vec::with_capacity(closes.len() + 1);
    prefix.push(0.0);
    for close in &closes {
        prefix.push(prefix.last().copied().unwrap_or_default() + close);
    }
    let rsi6 = rsi(&closes, 6);
    let rsi12 = rsi(&closes, 12);
    let rsi14 = rsi(&closes, 14);
    let (k, d) = kd(bars);
    let (dif, signal) = macd(&closes);

    bars.iter()
        .enumerate()
        .map(|(i, bar)| {
            let moving_averages = MA_PERIODS.map(|period| {
                (i + 1 >= period).then(|| (prefix[i + 1] - prefix[i + 1 - period]) / period as f64)
            });
            IndicatorPoint {
                date: bar.date,
                close: bar.close,
                moving_averages,
                rsi6: rsi6[i],
                rsi12: rsi12[i],
                rsi14: rsi14[i],
                k: k[i],
                d: d[i],
                macd_dif: dif[i],
                macd_signal: signal[i],
                macd_histogram: dif[i].zip(signal[i]).map(|(dif, signal)| dif - signal),
            }
        })
        .collect()
}

/// Wilder 平滑 RSI；第 `period` 日（0 起算）起才有值。
pub(super) fn rsi(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; closes.len()];
    let (mut avg_gain, mut avg_loss) = (0.0, 0.0);
    for i in 1..closes.len() {
        let change = closes[i] - closes[i - 1];
        let (gain, loss) = (change.max(0.0), (-change).max(0.0));
        if i <= period {
            avg_gain += gain / period as f64;
            avg_loss += loss / period as f64;
            if i < period {
                continue;
            }
        } else {
            avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
            avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
        }
        out[i] = Some(if avg_loss == 0.0 {
            if avg_gain == 0.0 { 50.0 } else { 100.0 }
        } else {
            100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
        });
    }
    out
}

/// KD(9,3,3)；第 9 日起才有值。
fn kd(bars: &[PriceBar]) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let mut k_out = vec![None; bars.len()];
    let mut d_out = vec![None; bars.len()];
    let (mut k, mut d) = (50.0, 50.0);
    for i in (KD_PERIOD - 1)..bars.len() {
        let window = &bars[i + 1 - KD_PERIOD..=i];
        let high = window.iter().map(bar_high).fold(f64::MIN, f64::max);
        let low = window.iter().map(bar_low).fold(f64::MAX, f64::min);
        let rsv = if high > low {
            (bars[i].close - low) / (high - low) * 100.0
        } else {
            50.0
        };
        k = k * 2.0 / 3.0 + rsv / 3.0;
        d = d * 2.0 / 3.0 + k / 3.0;
        k_out[i] = Some(k);
        d_out[i] = Some(d);
    }
    (k_out, d_out)
}

/// 最高價；缺值或低於收盤價時以收盤價代替。
pub(super) fn bar_high(bar: &PriceBar) -> f64 {
    bar.high.max(bar.close)
}

/// 最低價；缺值（≤ 0）或高於收盤價時以收盤價代替。
pub(super) fn bar_low(bar: &PriceBar) -> f64 {
    if bar.low > 0.0 {
        bar.low.min(bar.close)
    } else {
        bar.close
    }
}

/// MACD(12,26,9) 的 DIF 與訊號線；DIF 自第 26 日、訊號線自第 34 日起才有值。
pub(super) fn macd(closes: &[f64]) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let mut dif_out = vec![None; closes.len()];
    let mut signal_out = vec![None; closes.len()];
    let Some(&first) = closes.first() else {
        return (dif_out, signal_out);
    };
    let alpha = |period: usize| 2.0 / (period as f64 + 1.0);
    let (mut fast, mut slow) = (first, first);
    let mut signal: Option<f64> = None;
    for (i, &close) in closes.iter().enumerate() {
        fast += alpha(MACD_FAST) * (close - fast);
        slow += alpha(MACD_SLOW) * (close - slow);
        if i + 1 < MACD_SLOW {
            continue;
        }
        let dif = fast - slow;
        dif_out[i] = Some(dif);
        let next = signal.map_or(dif, |prev| prev + alpha(MACD_SIGNAL) * (dif - prev));
        signal = Some(next);
        if i + 1 >= MACD_SLOW + MACD_SIGNAL - 1 {
            signal_out[i] = Some(next);
        }
    }
    (dif_out, signal_out)
}
