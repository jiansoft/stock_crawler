//! 加權指數（TAIEX）的趨勢狀態。
//!
//! 由新到舊的收盤序列計算 5／20／60／120／240 日均線的位置與方向、均線多空排列、站在 60 日
//! 均線同一側的天數、52 週高低點與近 N 日漲跌幅，以及收盤穿越均線的日子。Data API 的
//! `/market/regime`、每週市場摘要與收盤後的大盤趨勢通知共用這裡的計算；數值不四捨五入，
//! 由呈現端決定位數。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 均線天數。
pub const MA_DAYS: [u16; 5] = [5, 20, 60, 120, 240];

/// 判斷均線方向時與幾個交易日前的均線比較。
pub const SLOPE_LAG: usize = 5;

/// 52 週約幾個交易日。
pub const WEEKS_52_DAYS: usize = 250;

/// 計算趨勢狀態需要讀取的交易日數：240 日均線往回數 [`SLOPE_LAG`] 天，以及站在 60 日均線
/// 同一側的天數都夠用。
pub const LOOKBACK: usize = 520;

/// 加權指數在 `index` 表的分類。
pub const TAIEX: &str = "TAIEX";

/// 加權指數相對一條均線的位置；資料不足時除天數外都是 `None`。
#[derive(Debug, Clone, PartialEq)]
pub struct MovingAverageState {
    /// 均線天數（交易日）。
    pub days: u16,
    /// 均線值。
    pub value: Option<f64>,
    /// 收盤相對均線的距離（%）。
    pub distance_percent: Option<f64>,
    /// 收盤是否在均線上方（含等於）。
    pub above: Option<bool>,
    /// 均線是否比 [`SLOPE_LAG`] 個交易日前高。
    pub rising: Option<bool>,
}

/// 收盤與 20／60／120／240 日均線的排列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// 收盤 > 20 > 60 > 120 > 240 日均線。
    Bullish,
    /// 收盤 < 20 < 60 < 120 < 240 日均線。
    Bearish,
    /// 其他，含資料不足。
    Mixed,
}

impl Alignment {
    /// API 使用的名稱。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bullish => "bullish",
            Self::Bearish => "bearish",
            Self::Mixed => "mixed",
        }
    }

    /// 中文說明。
    pub fn label(self) -> &'static str {
        match self {
            Self::Bullish => "多頭排列",
            Self::Bearish => "空頭排列",
            Self::Mixed => "均線糾結",
        }
    }
}

/// 加權指數在某交易日的趨勢狀態。
#[derive(Debug, Clone, PartialEq)]
pub struct MarketRegime {
    /// 資料日。
    pub date: NaiveDate,
    /// 收盤。
    pub close: f64,
    /// 與前一個交易日相比的漲跌點數。
    pub change: Option<f64>,
    /// 與前一個交易日相比的漲跌幅（%）。
    pub change_percent: Option<f64>,
    /// 均線排列。
    pub alignment: Alignment,
    /// 各均線的位置與方向，依 [`MA_DAYS`] 排列。
    pub moving_averages: Vec<MovingAverageState>,
    /// 收盤連續站在 60 日均線同一側的交易日數（含資料日）。
    pub days_on_side_of_ma60: Option<i64>,
    /// 近 [`WEEKS_52_DAYS`] 個交易日的最高收盤。
    pub high_52_week: f64,
    /// 近 [`WEEKS_52_DAYS`] 個交易日的最低收盤。
    pub low_52_week: f64,
    /// 收盤相對 52 週最高的距離（%）。
    pub from_high_percent: f64,
    /// 收盤相對 52 週最低的距離（%）。
    pub from_low_percent: f64,
    /// 近 5 個交易日漲跌幅（%）。
    pub return_5d: Option<f64>,
    /// 近 20 個交易日漲跌幅（%）。
    pub return_20d: Option<f64>,
    /// 近 60 個交易日漲跌幅（%）。
    pub return_60d: Option<f64>,
    /// 近 240 個交易日漲跌幅（%）。
    pub return_240d: Option<f64>,
}

impl MarketRegime {
    /// `days` 日均線的狀態；不是 [`MA_DAYS`] 之一回 `None`。
    pub fn moving_average(&self, days: u16) -> Option<&MovingAverageState> {
        self.moving_averages
            .iter()
            .find(|status| status.days == days)
    }
}

/// 收盤穿越均線的一天。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovingAverageCross {
    /// 穿越當天。
    pub date: NaiveDate,
    /// 均線天數。
    pub days: u16,
    /// `true` 為由下往上站上均線，`false` 為跌破。
    pub above: bool,
}

/// 指數收盤的查詢介面。
#[async_trait]
pub trait IndexCloseQuery: Send + Sync {
    /// 分類 `category` 在 `until`（含，`None` 為不限）之前最近 `limit` 個交易日的收盤，由新到舊。
    async fn fetch_index_closes(
        &self,
        category: &str,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<(NaiveDate, f64)>>;
}

/// 讀取 `until`（含）之前最近 [`LOOKBACK`] 個交易日的加權指數收盤，由新到舊。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub async fn load_taiex(
    query: &impl IndexCloseQuery,
    until: Option<NaiveDate>,
) -> Result<Vec<(NaiveDate, f64)>> {
    query.fetch_index_closes(TAIEX, until, LOOKBACK).await
}

/// 由新到舊的 `(日期, 收盤)` 算出最新一天的趨勢狀態；沒有任何資料回 `None`。
pub fn regime(points: &[(NaiveDate, f64)]) -> Option<MarketRegime> {
    let &(date, close) = points.first()?;
    let closes = closes_of(points);
    let percent = |from: f64| (from > 0.0).then(|| (close / from - 1.0) * 100.0);

    let moving_averages: Vec<MovingAverageState> = MA_DAYS
        .iter()
        .map(|&days| {
            let value = moving_average(&closes, usize::from(days), 0);
            let earlier = moving_average(&closes, usize::from(days), SLOPE_LAG);
            MovingAverageState {
                days,
                value,
                distance_percent: value.and_then(percent),
                above: value.map(|value| close >= value),
                rising: value.zip(earlier).map(|(now, before)| now > before),
            }
        })
        .collect();
    let average = |days: u16| {
        moving_averages
            .iter()
            .find(|status| status.days == days)
            .and_then(|status| status.value)
    };
    let alignment = alignment(
        close,
        [average(20), average(60), average(120), average(240)],
    );

    let window = &closes[..closes.len().min(WEEKS_52_DAYS)];
    let high = window.iter().copied().fold(f64::MIN, f64::max);
    let low = window.iter().copied().fold(f64::MAX, f64::min);
    let previous = closes.get(1).copied();
    let over = |days: usize| closes.get(days).copied().and_then(percent);

    Some(MarketRegime {
        date,
        close,
        change: previous.map(|previous| close - previous),
        change_percent: previous.and_then(percent),
        alignment,
        moving_averages,
        days_on_side_of_ma60: days_on_side(&closes, 60),
        high_52_week: high,
        low_52_week: low,
        from_high_percent: percent(high).unwrap_or(0.0),
        from_low_percent: percent(low).unwrap_or(0.0),
        return_5d: over(5),
        return_20d: over(20),
        return_60d: over(60),
        return_240d: over(240),
    })
}

/// 最近 `within` 個交易日內，收盤穿越 `days` 日均線的日子，由新到舊。
///
/// 某天收盤與均線的相對位置（上方含等於）和前一個交易日不同就算穿越；前一個交易日沒有均線的
/// 不算。
pub fn crosses(points: &[(NaiveDate, f64)], days: u16, within: usize) -> Vec<MovingAverageCross> {
    let closes = closes_of(points);
    let days_usize = usize::from(days);
    let side = |offset: usize| {
        moving_average(&closes, days_usize, offset).map(|average| closes[offset] >= average)
    };
    (0..within.min(points.len()))
        .filter_map(|offset| {
            let today = side(offset)?;
            let before = side(offset + 1)?;
            (today != before).then(|| MovingAverageCross {
                date: points[offset].0,
                days,
                above: today,
            })
        })
        .collect()
}

/// 只取收盤。
fn closes_of(points: &[(NaiveDate, f64)]) -> Vec<f64> {
    points.iter().map(|(_, close)| *close).collect()
}

/// 由新到舊的收盤從第 `offset` 個起算的 `days` 日平均；資料不足回 `None`。
fn moving_average(closes: &[f64], days: usize, offset: usize) -> Option<f64> {
    let window = closes.get(offset..offset + days)?;
    Some(window.iter().sum::<f64>() / days as f64)
}

/// 收盤與 20／60／120／240 日均線由高到低為多頭、由低到高為空頭，其他（含資料不足）為糾結。
fn alignment(close: f64, averages: [Option<f64>; 4]) -> Alignment {
    let Some(values) = averages
        .into_iter()
        .collect::<Option<Vec<f64>>>()
        .map(|averages| std::iter::once(close).chain(averages).collect::<Vec<f64>>())
    else {
        return Alignment::Mixed;
    };
    if values.windows(2).all(|pair| pair[0] > pair[1]) {
        Alignment::Bullish
    } else if values.windows(2).all(|pair| pair[0] < pair[1]) {
        Alignment::Bearish
    } else {
        Alignment::Mixed
    }
}

/// 收盤連續站在 `days` 日均線同一側的交易日數（含最新一天）；最新一天沒有均線回 `None`。
fn days_on_side(closes: &[f64], days: usize) -> Option<i64> {
    let side = |offset: usize| {
        moving_average(closes, days, offset).map(|average| closes[offset] >= average)
    };
    let today = side(0)?;
    let count = (0..closes.len())
        .map_while(side)
        .take_while(|above| *above == today)
        .count();
    Some(count as i64)
}

#[cfg(test)]
mod tests;
