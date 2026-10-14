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

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 均線週期：週、雙週、月、季、半年、年線。
pub const MA_PERIODS: [usize; 6] = [5, 10, 20, 60, 120, 240];

/// 計算前額外讀取的交易日數：涵蓋年線所需的 240 日，並讓 EMA、KD 的起始值影響衰減。
pub const WARMUP_BARS: usize = 360;

/// 52 週新高／新低訊號的來源名稱。
pub const FIFTY_TWO_WEEK_INDICATOR: &str = "52_week";

/// 52 週回看的日曆天數（不含當日），與市場廣度統計 `daily_stock_price_stats` 一致。
const FIFTY_TWO_WEEK_DAYS: u64 = 364;

/// 前 52 週至少要有的交易日數；不足（例如新上市）不判斷新高新低，與市場廣度統計一致。
const FIFTY_TWO_WEEK_MIN_PRIOR_DAYS: usize = 200;

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
    fn factor(&self, prev_close: f64) -> Option<f64> {
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

/// 某股票某交易日的技術訊號（收盤後預先計算，供全市場訊號選股查詢）。
#[derive(Debug, Clone, PartialEq)]
pub struct StockTechnicalSignal {
    /// 交易日。
    pub date: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 訊號內容。
    pub signal: TechnicalSignal,
}

/// 全市場技術訊號的存取。
#[async_trait]
pub trait TechnicalSignalRepository: Send + Sync {
    /// 指定交易日有成交、未暫停交易的上市櫃股票代號。
    async fn fetch_signal_symbols(&self, date: NaiveDate) -> Result<Vec<String>>;

    /// 以 `signals` 覆蓋指定交易日的全部訊號（同一交易內先刪後寫）。
    async fn replace_signals(
        &self,
        date: NaiveDate,
        signals: &[StockTechnicalSignal],
    ) -> Result<()>;
}

/// 讀取已預先計算的技術訊號。
#[async_trait]
pub trait TechnicalSignalQuery: Send + Sync {
    /// 指定交易日、指定股票的訊號，依代號排序；資料表中無法辨識的來源或種類略過。
    async fn fetch_signals(
        &self,
        date: NaiveDate,
        stock_symbols: &[String],
    ) -> Result<Vec<StockTechnicalSignal>>;

    /// `from`～`to`（含）期間 `indicators` 來源的訊號，依來源與種類統計出現後
    /// [`OUTCOME_HORIZON`] 個交易日的還原報酬（全市場；尚未滿天期的不計）。
    async fn fetch_signal_outcomes(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        indicators: &[&str],
    ) -> Result<Vec<SignalOutcome>>;
}

/// [`SignalOutcome`] 統計的天期（交易日數）。
pub const OUTCOME_HORIZON: usize = 20;

/// 某種訊號在一段期間內出現後 [`OUTCOME_HORIZON`] 個交易日的表現（全市場）。
#[derive(Debug, Clone, PartialEq)]
pub struct SignalOutcome {
    /// 訊號來源。
    pub indicator: String,
    /// 訊號種類（[`SignalKind::as_str`]）。
    pub kind: String,
    /// 樣本數（已滿天期的訊號次數）。
    pub samples: i64,
    /// 平均還原報酬（%）。
    pub avg_return: f64,
    /// 上漲比例（0–1）。
    pub win_rate: f64,
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
    /// 當日最高價創前 52 週新高。
    NewHigh,
    /// 當日最低價創前 52 週新低。
    NewLow,
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
            Self::NewHigh => "new_high",
            Self::NewLow => "new_low",
        }
    }

    /// 由對外名稱解析；未知名稱回 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        SIGNAL_KINDS.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// 全部訊號種類，依對外說明的順序。
pub const SIGNAL_KINDS: [SignalKind; 8] = [
    SignalKind::GoldenCross,
    SignalKind::DeathCross,
    SignalKind::CrossAbove,
    SignalKind::CrossBelow,
    SignalKind::Overbought,
    SignalKind::Oversold,
    SignalKind::NewHigh,
    SignalKind::NewLow,
];

/// 全部訊號來源（[`TechnicalSignal::indicator`]），依對外說明的順序。
pub const SIGNAL_INDICATORS: [&str; 9] = [
    "ma5_ma20",
    "ma20_ma60",
    "kd",
    "macd",
    "close_ma20",
    "close_ma60",
    "close_ma240",
    "rsi14",
    FIFTY_TWO_WEEK_INDICATOR,
];

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

impl TechnicalSignal {
    /// 由資料表的來源與種類名稱還原；任一名稱不在 [`SIGNAL_INDICATORS`]／[`SIGNAL_KINDS`] 時回 `None`。
    pub fn from_names(indicator: &str, kind: &str, description: String) -> Option<Self> {
        Some(Self {
            indicator: SIGNAL_INDICATORS
                .into_iter()
                .find(|known| *known == indicator)?,
            kind: SignalKind::parse(kind)?,
            description,
        })
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

/// 最新一日（`bars` 最後一根）是否創 52 週新高或新低。
///
/// 判斷規則與市場廣度統計 `daily_stock_price_stats` 相同：當日最高價高於前 364 天（不含當日）
/// 的最高價算新高；當日最低價低於前 364 天大於 0 的最低價算新低；前 364 天不足 200 個
/// 交易日（例如新上市）不判斷；不以收盤價替代缺值。差別是這裡用的是呼叫端傳入的日線
/// （訊號計算時已還原權值），市場廣度統計用原始價格。
pub fn fifty_two_week_signals(bars: &[PriceBar]) -> Vec<TechnicalSignal> {
    let Some((last, prior)) = bars.split_last() else {
        return Vec::new();
    };
    let from = last.date - chrono::Days::new(FIFTY_TWO_WEEK_DAYS);
    let window: Vec<&PriceBar> = prior
        .iter()
        .filter(|bar| bar.date >= from && bar.date < last.date)
        .collect();
    if window.len() < FIFTY_TWO_WEEK_MIN_PRIOR_DAYS {
        return Vec::new();
    }

    let mut out = Vec::new();
    let prior_high = window.iter().map(|bar| bar.high).fold(f64::MIN, f64::max);
    if last.high > prior_high {
        out.push(TechnicalSignal {
            indicator: FIFTY_TWO_WEEK_INDICATOR,
            kind: SignalKind::NewHigh,
            description: format!(
                "最高價 {} 創 52 週新高（前高 {}）",
                price_text(last.high),
                price_text(prior_high)
            ),
        });
    }
    let prior_low = window
        .iter()
        .map(|bar| bar.low)
        .filter(|low| *low > 0.0)
        .fold(f64::MAX, f64::min);
    if last.low > 0.0 && last.low < prior_low && prior_low < f64::MAX {
        out.push(TechnicalSignal {
            indicator: FIFTY_TWO_WEEK_INDICATOR,
            kind: SignalKind::NewLow,
            description: format!(
                "最低價 {} 創 52 週新低（前低 {}）",
                price_text(last.low),
                price_text(prior_low)
            ),
        });
    }
    out
}

/// 價格四捨五入到小數兩位後去掉多餘的 0（還原後的價格可能有很長的小數）。
fn price_text(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
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

    /// 對外名稱固定，且可由名稱解析回原值。
    #[test]
    fn signal_kind_names_are_stable() {
        let names = SIGNAL_KINDS.map(SignalKind::as_str);
        assert_eq!(
            names,
            [
                "golden_cross",
                "death_cross",
                "cross_above",
                "cross_below",
                "overbought",
                "oversold",
                "new_high",
                "new_low"
            ]
        );
        for kind in SIGNAL_KINDS {
            assert_eq!(SignalKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SignalKind::parse("unknown"), None);
    }

    /// 資料表的名稱可還原成訊號；未知名稱回 None。
    #[test]
    fn technical_signal_from_names() {
        let signal = TechnicalSignal::from_names("52_week", "new_high", "說明".to_owned())
            .expect("已知名稱");
        assert_eq!(signal.indicator, FIFTY_TWO_WEEK_INDICATOR);
        assert_eq!(signal.kind, SignalKind::NewHigh);
        assert_eq!(signal.description, "說明");
        assert!(TechnicalSignal::from_names("ma1", "new_high", String::new()).is_none());
        assert!(TechnicalSignal::from_names("kd", "breakout", String::new()).is_none());
    }

    /// 前 52 週的日線：每 7 天 5 個交易日、共 `days` 根，最高 10、最低 5，最後加上當日。
    fn year_bars(days: usize, last_high: f64, last_low: f64) -> Vec<PriceBar> {
        let start = NaiveDate::from_ymd_opt(2025, 1, 6).expect("有效日期");
        let mut out: Vec<PriceBar> = (0..days)
            .map(|i| PriceBar {
                date: start + chrono::Days::new((i / 5 * 7 + i % 5) as u64),
                close: 8.0,
                high: 10.0,
                low: 5.0,
            })
            .collect();
        let last_date = out.last().expect("至少一根").date + chrono::Days::new(1);
        out.push(PriceBar {
            date: last_date,
            close: 8.0,
            high: last_high,
            low: last_low,
        });
        out
    }

    /// 當日最高價高於前 52 週最高價為新高、最低價低於前 52 週最低價為新低；等於不算。
    #[test]
    fn fifty_two_week_signals_compare_with_prior_year() {
        let both = fifty_two_week_signals(&year_bars(250, 10.5, 4.5));
        let kinds: Vec<SignalKind> = both.iter().map(|signal| signal.kind).collect();
        assert_eq!(kinds, [SignalKind::NewHigh, SignalKind::NewLow]);
        assert!(both.iter().all(|signal| signal.indicator == "52_week"));
        assert_eq!(both[0].description, "最高價 10.5 創 52 週新高（前高 10）");
        assert_eq!(both[1].description, "最低價 4.5 創 52 週新低（前低 5）");

        assert!(fifty_two_week_signals(&year_bars(250, 10.0, 5.0)).is_empty());
        assert!(fifty_two_week_signals(&[]).is_empty());
    }

    /// 前 52 週不足 200 個交易日不判斷；超過 364 天的舊資料不列入比較。
    #[test]
    fn fifty_two_week_signals_need_enough_recent_history() {
        assert!(fifty_two_week_signals(&year_bars(199, 99.0, 1.0)).is_empty());
        assert_eq!(fifty_two_week_signals(&year_bars(200, 99.0, 1.0)).len(), 2);

        // 一年多以前的極端高低價不影響：只看前 364 天。
        let mut bars = year_bars(300, 10.5, 4.5);
        bars[0].high = 50.0;
        bars[0].low = 1.0;
        assert_eq!(fifty_two_week_signals(&bars).len(), 2);
    }

    /// 最低價為 0（缺值）的日子不參與新低比較；當日最低價為 0 也不算新低。
    #[test]
    fn fifty_two_week_low_ignores_missing_prices() {
        let mut bars = year_bars(250, 10.0, 4.5);
        let len = bars.len();
        bars[len - 2].low = 0.0;
        let signals = fifty_two_week_signals(&bars);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind, SignalKind::NewLow);

        let zero_low = year_bars(250, 10.0, 0.0);
        assert!(fifty_two_week_signals(&zero_low).is_empty());
    }

    fn adjustment(date: NaiveDate, cash: f64, stock: f64, share_ratio: f64) -> PriceAdjustment {
        PriceAdjustment {
            date,
            cash_dividend: cash,
            stock_dividend: stock,
            share_ratio,
        }
    }

    /// 還原因子＝參考價 ÷ 前收：除息、除權、減資與分割；不合理或不需調整時為 None。
    #[test]
    fn price_adjustment_factor_follows_reference_price() {
        let factor = |cash, stock, ratio| adjustment(day(0), cash, stock, ratio).factor(100.0);
        assert_eq!(factor(5.0, 0.0, 1.0), Some(0.95));
        // 配股 1 元（每股配 0.1 股）：參考價 100 ÷ 1.1。
        assert!((factor(0.0, 1.0, 1.0).expect("有因子") - 1.0 / 1.1).abs() < 1e-12);
        // 減資五成（1 股變 0.5 股）價格加倍；1 拆 4 價格變四分之一。
        assert_eq!(factor(0.0, 0.0, 0.5), Some(2.0));
        assert_eq!(factor(0.0, 0.0, 4.0), Some(0.25));
        assert_eq!(factor(0.0, 0.0, 1.0), None);
        assert_eq!(factor(100.0, 0.0, 1.0), None);
        assert_eq!(factor(0.0, 0.0, 0.0), None);
        assert_eq!(adjustment(day(0), 1.0, 0.0, 1.0).factor(0.0), None);
    }

    /// 事件日之前的價格乘上因子、最新一根不變；多個事件累乘；
    /// 截止日之後與第一根（含）之前的事件不採用；事件日沒成交時套用到前一根（含）之前。
    #[test]
    fn adjust_bars_scales_prices_before_events() {
        let raw = bars(&[10.0, 10.0, 9.0, 9.0, 18.0]);
        let events = [
            adjustment(day(2), 1.0, 0.0, 1.0),
            adjustment(day(4), 0.0, 0.0, 0.5),
            adjustment(day(9), 3.0, 0.0, 1.0),
            adjustment(day(0), 3.0, 0.0, 1.0),
        ];
        // 除息 1 元：前收 10 → 參考價 9（0.9）；減資五成：前收 9 → 18（2 倍）。
        assert_eq!(adjustment_factors(&raw, &events), [1.8, 1.8, 2.0, 2.0, 1.0]);
        assert!(adjustment_factors(&[], &events).is_empty());
        let adjusted = adjust_bars(&raw, &events);
        let closes: Vec<f64> = adjusted.iter().map(|bar| bar.close).collect();
        assert_eq!(closes, [18.0, 18.0, 18.0, 18.0, 18.0]);
        assert_eq!(adjusted[0].high, 18.0);
        assert_eq!(adjusted[0].low, 18.0);
        assert!(adjusted.iter().zip(&raw).all(|(a, r)| a.date == r.date));

        // 事件日（day 2）沒有成交：以 day 1 收盤計算，day 1（含）之前調整。
        let gap = vec![raw[0], raw[1], raw[3]];
        let adjusted = adjust_bars(&gap, &[adjustment(day(2), 1.0, 0.0, 1.0)]);
        let closes: Vec<f64> = adjusted.iter().map(|bar| bar.close).collect();
        assert_eq!(closes, [9.0, 9.0, 9.0]);

        assert!(adjust_bars(&[], &[adjustment(day(2), 1.0, 0.0, 1.0)]).is_empty());
        assert_eq!(adjust_bars(&raw, &[]), raw);
    }

    /// 還原後除息跳空不再被誤判成 52 週新低；價格描述四捨五入到小數兩位。
    #[test]
    fn adjusted_bars_avoid_false_fifty_two_week_low() {
        let mut raw = year_bars(250, 10.0, 7.0);
        let ex_date = raw.last().expect("至少一根").date;
        // 除息 3 元：前收 8 → 參考價 5。當日最低 4.9 低於未還原的前低 5，
        // 還原後前低為 5 × 5/8 = 3.125，不算新低。
        raw.last_mut().expect("至少一根").low = 4.9;
        let unadjusted = fifty_two_week_signals(&raw);
        assert!(unadjusted.iter().any(|s| s.kind == SignalKind::NewLow));

        let adjusted = adjust_bars(&raw, &[adjustment(ex_date, 3.0, 0.0, 1.0)]);
        assert!(
            !fifty_two_week_signals(&adjusted)
                .iter()
                .any(|s| s.kind == SignalKind::NewLow)
        );

        assert_eq!(price_text(3.126), "3.13");
        assert_eq!(price_text(256.5), "256.5");
        assert_eq!(price_text(100.0), "100");
        assert_eq!(price_text(1.0 / 3.0), "0.33");
    }
}
