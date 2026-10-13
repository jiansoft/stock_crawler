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
//! 價格未還原除權息，除權息日會出現跳空；樣本不足的指標為 `None`。KD、RSI 與 EMA 都
//! 帶有遞迴記憶，呼叫端應多取 [`WARMUP_BARS`] 日讓起始值的影響衰減到可忽略。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 均線週期：週、雙週、月、季、半年、年線。
pub const MA_PERIODS: [usize; 6] = [5, 10, 20, 60, 120, 240];

/// 計算前額外讀取的交易日數：涵蓋年線所需的 240 日，並讓 EMA、KD 的起始值影響衰減。
pub const WARMUP_BARS: usize = 360;

const KD_PERIOD: usize = 9;
const MACD_FAST: usize = 12;
const MACD_SLOW: usize = 26;
const MACD_SIGNAL: usize = 9;
const RSI_OVERBOUGHT: f64 = 70.0;
const RSI_OVERSOLD: f64 = 30.0;
const KD_OVERBOUGHT: f64 = 80.0;
const KD_OVERSOLD: f64 = 20.0;

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

/// 技術指標的日線來源。
#[async_trait]
pub trait PriceBarRepository: Send + Sync {
    /// `until`（含，`None` 為不限）之前最近 `limit` 個有成交的交易日，依日期由舊到新。
    async fn fetch_price_bars(
        &self,
        stock_symbol: &str,
        until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<PriceBar>>;
}

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

/// 訊號種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// 短線向上穿越長線。
    GoldenCross,
    /// 短線向下穿越長線。
    DeathCross,
    /// 收盤價由下往上站上均線。
    CrossAbove,
    /// 收盤價由上往下跌破均線。
    CrossBelow,
    /// 指標位於超買區。
    Overbought,
    /// 指標位於超賣區。
    Oversold,
}

impl SignalKind {
    /// 對外的 snake_case 名稱。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GoldenCross => "golden_cross",
            Self::DeathCross => "death_cross",
            Self::CrossAbove => "cross_above",
            Self::CrossBelow => "cross_below",
            Self::Overbought => "overbought",
            Self::Oversold => "oversold",
        }
    }
}

/// 最新一日出現的技術訊號。
#[derive(Debug, Clone, PartialEq)]
pub struct TechnicalSignal {
    /// 訊號來源，例如 `ma5_ma20`、`kd`、`macd`、`close_ma60`、`rsi14`。
    pub indicator: &'static str,
    /// 訊號種類。
    pub kind: SignalKind,
    /// 中文說明。
    pub description: String,
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
fn rsi(closes: &[f64], period: usize) -> Vec<Option<f64>> {
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
fn bar_high(bar: &PriceBar) -> f64 {
    bar.high.max(bar.close)
}

/// 最低價；缺值（≤ 0）或高於收盤價時以收盤價代替。
fn bar_low(bar: &PriceBar) -> f64 {
    if bar.low > 0.0 {
        bar.low.min(bar.close)
    } else {
        bar.close
    }
}

/// MACD(12,26,9) 的 DIF 與訊號線；DIF 自第 26 日、訊號線自第 34 日起才有值。
fn macd(closes: &[f64]) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
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

/// 兩條線的交叉：`Some(true)` 向上穿越、`Some(false)` 向下穿越、其餘為 `None`。
fn crossing(prev: (Option<f64>, Option<f64>), last: (Option<f64>, Option<f64>)) -> Option<bool> {
    let (prev_a, prev_b, a, b) = (prev.0?, prev.1?, last.0?, last.1?);
    if prev_a <= prev_b && a > b {
        Some(true)
    } else if prev_a >= prev_b && a < b {
        Some(false)
    } else {
        None
    }
}

/// 均線的中文俗稱。
fn ma_name(period: usize) -> String {
    let alias = match period {
        5 => "（週線）",
        20 => "（月線）",
        60 => "（季線）",
        240 => "（年線）",
        _ => "",
    };
    format!("{period} 日均線{alias}")
}

/// 最新一日（`last`）相對前一日（`prev`）的交叉訊號，以及 RSI、KD 的超買超賣。
pub fn signals(prev: Option<&IndicatorPoint>, last: &IndicatorPoint) -> Vec<TechnicalSignal> {
    let mut out = Vec::new();
    if let Some(prev) = prev {
        for (indicator, short, long) in [("ma5_ma20", 5, 20), ("ma20_ma60", 20, 60)] {
            let pair = |p: &IndicatorPoint| (p.moving_average(short), p.moving_average(long));
            if let Some(up) = crossing(pair(prev), pair(last)) {
                out.push(cross_signal(
                    indicator,
                    up,
                    format!("{}{}穿越 {}", ma_name(short), direction(up), ma_name(long)),
                ));
            }
        }
        let kd = |p: &IndicatorPoint| (p.k, p.d);
        if let (Some(up), Some(k)) = (crossing(kd(prev), kd(last)), last.k) {
            out.push(cross_signal(
                "kd",
                up,
                format!("K 值{}穿越 D 值，K＝{k:.2}", direction(up)),
            ));
        }
        let macd = |p: &IndicatorPoint| (p.macd_dif, p.macd_signal);
        if let Some(up) = crossing(macd(prev), macd(last)) {
            out.push(cross_signal(
                "macd",
                up,
                format!("MACD 的 DIF {}穿越訊號線", direction(up)),
            ));
        }
        for (indicator, period) in [("close_ma20", 20), ("close_ma60", 60), ("close_ma240", 240)] {
            let pair = |p: &IndicatorPoint| (Some(p.close), p.moving_average(period));
            if let Some(up) = crossing(pair(prev), pair(last)) {
                let (kind, verb) = if up {
                    (SignalKind::CrossAbove, "站上")
                } else {
                    (SignalKind::CrossBelow, "跌破")
                };
                out.push(TechnicalSignal {
                    indicator,
                    kind,
                    description: format!("收盤價{verb} {}", ma_name(period)),
                });
            }
        }
    }
    if let Some(rsi) = last.rsi14 {
        push_level(
            &mut out,
            "rsi14",
            "RSI(14)",
            rsi,
            RSI_OVERBOUGHT,
            RSI_OVERSOLD,
        );
    }
    if let Some(k) = last.k {
        push_level(&mut out, "kd", "K 值", k, KD_OVERBOUGHT, KD_OVERSOLD);
    }
    out
}

/// 「向上」或「向下」。
fn direction(up: bool) -> &'static str {
    if up { "向上" } else { "向下" }
}

/// 黃金交叉或死亡交叉訊號。
fn cross_signal(indicator: &'static str, up: bool, description: String) -> TechnicalSignal {
    let (kind, name) = if up {
        (SignalKind::GoldenCross, "黃金交叉")
    } else {
        (SignalKind::DeathCross, "死亡交叉")
    };
    TechnicalSignal {
        indicator,
        kind,
        description: format!("{description}（{name}）"),
    }
}

/// 指標落在超買（≥ `high`）或超賣（≤ `low`）區時加入訊號。
fn push_level(
    out: &mut Vec<TechnicalSignal>,
    indicator: &'static str,
    label: &str,
    value: f64,
    high: f64,
    low: f64,
) {
    let (kind, zone, bound) = if value >= high {
        (SignalKind::Overbought, "超買", high)
    } else if value <= low {
        (SignalKind::Oversold, "超賣", low)
    } else {
        return;
    };
    out.push(TechnicalSignal {
        indicator,
        kind,
        description: format!("{label} {value:.2}，位於{zone}區（門檻 {bound}）"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(offset: usize) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).expect("有效日期") + chrono::Days::new(offset as u64)
    }

    /// 只有收盤價的日線（最高、最低等於收盤）。
    fn bars(closes: &[f64]) -> Vec<PriceBar> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &close)| PriceBar {
                date: day(i),
                close,
                high: close,
                low: close,
            })
            .collect()
    }

    fn approx(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("應有值");
        assert!(
            (actual - expected).abs() < 1e-9,
            "actual {actual} expected {expected}"
        );
    }

    fn point(close: f64) -> IndicatorPoint {
        IndicatorPoint {
            date: day(0),
            close,
            moving_averages: [None; MA_PERIODS.len()],
            rsi6: None,
            rsi12: None,
            rsi14: None,
            k: None,
            d: None,
            macd_dif: None,
            macd_signal: None,
            macd_histogram: None,
        }
    }

    /// 均線在滿 N 日後才有值，乖離率以收盤相對均線計算。
    #[test]
    fn moving_averages_need_full_window() {
        let closes: Vec<f64> = (1..=20).map(f64::from).collect();
        let points = compute(&bars(&closes));
        assert_eq!(points.len(), 20);
        assert!(points[3].moving_average(5).is_none());
        approx(points[4].moving_average(5), 3.0);
        approx(points[19].moving_average(5), 18.0);
        approx(points[19].moving_average(10), 15.5);
        approx(points[19].moving_average(20), 10.5);
        assert!(points[19].moving_average(60).is_none());
        assert!(points[19].moving_average(7).is_none());
        approx(points[19].bias(5), (20.0 - 18.0) / 18.0 * 100.0);
        assert!(points[19].bias(60).is_none());
        assert!(compute(&[]).is_empty());
    }

    /// RSI 首值為簡單平均，之後 Wilder 平滑；只漲為 100、全平為 50。
    #[test]
    fn rsi_uses_wilder_smoothing() {
        // 漲跌：+2, -1, +2, -1, +2, -1, -3
        let closes = [10.0, 12.0, 11.0, 13.0, 12.0, 14.0, 13.0, 10.0];
        let values = rsi(&closes, 6);
        assert!(values[5].is_none());
        // 前 6 日平均漲 1、平均跌 0.5 → RS 2 → 66.67
        approx(values[6], 100.0 - 100.0 / 3.0);
        // 第 7 日跌 3：平均漲 5/6、平均跌 (0.5×5+3)/6
        let (gain, loss) = (5.0 / 6.0, (0.5 * 5.0 + 3.0) / 6.0);
        approx(values[7], 100.0 - 100.0 / (1.0 + gain / loss));

        let rising: Vec<f64> = (1..=8).map(f64::from).collect();
        approx(rsi(&rising, 6)[7], 100.0);
        approx(rsi(&[5.0; 8], 6)[7], 50.0);
    }

    /// KD 自第 9 日起有值；高低相同取 RSV 50，缺高低價時以收盤代替。
    #[test]
    fn kd_follows_rsv_smoothing() {
        let flat = compute(&bars(&[10.0; 9]));
        assert!(flat[7].k.is_none());
        approx(flat[8].k, 50.0);
        approx(flat[8].d, 50.0);

        // 九日區間 1～9，收在最高 → RSV 100 → K 66.67、D 55.56
        let mut rising = bars(&(1..=9).map(f64::from).collect::<Vec<_>>());
        rising[3].high = 0.0;
        rising[4].low = 0.0;
        let points = compute(&rising);
        approx(points[8].k, 50.0 * 2.0 / 3.0 + 100.0 / 3.0);
        approx(points[8].d, 50.0 * 2.0 / 3.0 + (200.0 / 3.0) / 3.0);
    }

    /// 缺值的最高、最低價以收盤價代替。
    #[test]
    fn bar_extremes_fall_back_to_close() {
        let bar = PriceBar {
            date: day(0),
            close: 10.0,
            high: 0.0,
            low: 0.0,
        };
        assert_eq!((bar_high(&bar), bar_low(&bar)), (10.0, 10.0));
        let wide = PriceBar {
            high: 12.0,
            low: 9.0,
            ..bar
        };
        assert_eq!((bar_high(&wide), bar_low(&wide)), (12.0, 9.0));
    }

    /// MACD：DIF 自第 26 日、訊號線自第 34 日起有值；平盤時都是 0。
    #[test]
    fn macd_warms_up_before_reporting() {
        let flat = compute(&bars(&[10.0; 40]));
        assert!(flat[24].macd_dif.is_none());
        approx(flat[25].macd_dif, 0.0);
        assert!(flat[32].macd_signal.is_none());
        approx(flat[33].macd_signal, 0.0);
        approx(flat[39].macd_histogram, 0.0);
        assert!(flat[32].macd_histogram.is_none());

        // 持續上漲時快線在慢線之上
        let rising: Vec<f64> = (1..=40).map(f64::from).collect();
        let points = compute(&bars(&rising));
        assert!(points[39].macd_dif.expect("有 DIF") > 0.0);

        // 不足 26 日沒有 DIF；空輸入回空結果
        let (dif, _) = macd(&[10.0, 12.0]);
        assert!(dif.iter().all(Option::is_none));
        assert!(macd(&[]).0.is_empty());
    }

    /// 均線、KD、MACD 的交叉與收盤價站上／跌破均線。
    #[test]
    fn signals_detect_crosses() {
        let mut prev = point(100.0);
        let mut last = point(110.0);
        // MA5 由 ≤ MA20 變 >：黃金交叉；MA20 由 ≥ MA60 變 <：死亡交叉
        prev.moving_averages = [
            Some(99.0),
            None,
            Some(100.0),
            Some(100.0),
            None,
            Some(105.0),
        ];
        last.moving_averages = [
            Some(102.0),
            None,
            Some(101.0),
            Some(102.0),
            None,
            Some(108.0),
        ];
        prev.k = Some(15.0);
        prev.d = Some(18.0);
        last.k = Some(22.0);
        last.d = Some(20.0);
        prev.macd_dif = Some(1.0);
        prev.macd_signal = Some(0.5);
        last.macd_dif = Some(0.2);
        last.macd_signal = Some(0.4);

        let found = signals(Some(&prev), &last);
        let kinds: Vec<(&str, SignalKind)> = found.iter().map(|s| (s.indicator, s.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("ma5_ma20", SignalKind::GoldenCross),
                ("ma20_ma60", SignalKind::DeathCross),
                ("kd", SignalKind::GoldenCross),
                ("macd", SignalKind::DeathCross),
                ("close_ma20", SignalKind::CrossAbove),
                ("close_ma60", SignalKind::CrossAbove),
                ("close_ma240", SignalKind::CrossAbove),
            ]
        );
        assert_eq!(
            found[0].description,
            "5 日均線（週線）向上穿越 20 日均線（月線）（黃金交叉）"
        );
        assert_eq!(
            found[2].description,
            "K 值向上穿越 D 值，K＝22.00（黃金交叉）"
        );
        assert_eq!(found[4].description, "收盤價站上 20 日均線（月線）");

        // 反向：收盤跌破
        let below = signals(Some(&last), &{
            let mut p = last.clone();
            p.close = 90.0;
            p
        });
        assert!(
            below
                .iter()
                .any(|s| s.indicator == "close_ma20" && s.kind == SignalKind::CrossBelow)
        );
        assert_eq!(ma_name(120), "120 日均線");
    }

    /// 沒有前一日只看超買超賣；RSI 與 K 值的門檻含等號。
    #[test]
    fn signals_report_overbought_and_oversold() {
        let mut last = point(10.0);
        last.rsi14 = Some(70.0);
        last.k = Some(20.0);
        let found = signals(None, &last);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].indicator, found[0].kind),
            ("rsi14", SignalKind::Overbought)
        );
        assert_eq!(found[0].description, "RSI(14) 70.00，位於超買區（門檻 70）");
        assert_eq!(
            (found[1].indicator, found[1].kind),
            ("kd", SignalKind::Oversold)
        );

        last.rsi14 = Some(50.0);
        last.k = Some(50.0);
        assert!(signals(None, &last).is_empty());
        assert!(signals(Some(&point(10.0)), &point(10.0)).is_empty());
    }

    /// 對外名稱固定。
    #[test]
    fn signal_kind_names_are_stable() {
        let names = [
            SignalKind::GoldenCross,
            SignalKind::DeathCross,
            SignalKind::CrossAbove,
            SignalKind::CrossBelow,
            SignalKind::Overbought,
            SignalKind::Oversold,
        ]
        .map(SignalKind::as_str);
        assert_eq!(
            names,
            [
                "golden_cross",
                "death_cross",
                "cross_above",
                "cross_below",
                "overbought",
                "oversold"
            ]
        );
    }
}
