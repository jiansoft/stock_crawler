//! 訊號判斷：均線、KD、MACD 交叉，收盤價站上或跌破均線，RSI／KD 超買超賣，以及 52 週新高新低。

use super::FIFTY_TWO_WEEK_INDICATOR;
use super::bars::PriceBar;
use super::indicators::IndicatorPoint;
use super::signal::{SignalKind, TechnicalSignal};

/// 52 週回看的日曆天數（不含當日），與市場廣度統計 `daily_stock_price_stats` 一致。
const FIFTY_TWO_WEEK_DAYS: u64 = 364;

/// 前 52 週至少要有的交易日數；不足（例如新上市）不判斷新高新低，與市場廣度統計一致。
const FIFTY_TWO_WEEK_MIN_PRIOR_DAYS: usize = 200;

const RSI_OVERBOUGHT: f64 = 70.0;
const RSI_OVERSOLD: f64 = 30.0;
const KD_OVERBOUGHT: f64 = 80.0;
const KD_OVERSOLD: f64 = 20.0;

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
pub(super) fn ma_name(period: usize) -> String {
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
pub(super) fn price_text(value: f64) -> String {
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
