//! 技術訊號的型別、來源與種類清單，以及預先計算訊號的存取介面。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::{FIFTY_TWO_WEEK_INDICATOR, RANGE_BREAKOUT_INDICATOR, RS50_INDICATOR, RS80_INDICATOR};

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

    /// `from`～`to`（含）期間有成交的上市櫃股票代號，含之後下市的（歷史回補不能有存活者偏差）。
    async fn fetch_traded_symbols(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<String>>;

    /// 以 `signals` 覆蓋 `indicator` 來源在 `from`～`to`（含）的全部訊號（同一交易內先刪後寫），
    /// 其他來源不動；`signals` 須全部是該來源、日期在期間內。
    async fn replace_indicator_signals(
        &self,
        indicator: &str,
        from: NaiveDate,
        to: NaiveDate,
        signals: &[StockTechnicalSignal],
    ) -> Result<()>;

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
    /// 整理區間後收盤帶量突破區間高點。
    Breakout,
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
            Self::Breakout => "breakout",
        }
    }

    /// 由對外名稱解析；未知名稱回 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        SIGNAL_KINDS.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// 全部訊號種類，依對外說明的順序。
pub const SIGNAL_KINDS: [SignalKind; 9] = [
    SignalKind::GoldenCross,
    SignalKind::DeathCross,
    SignalKind::CrossAbove,
    SignalKind::CrossBelow,
    SignalKind::Overbought,
    SignalKind::Oversold,
    SignalKind::NewHigh,
    SignalKind::NewLow,
    SignalKind::Breakout,
];

/// 全部訊號來源（[`TechnicalSignal::indicator`]），依對外說明的順序。
pub const SIGNAL_INDICATORS: [&str; 12] = [
    "ma5_ma20",
    "ma20_ma60",
    "kd",
    "macd",
    "close_ma20",
    "close_ma60",
    "close_ma240",
    "rsi14",
    FIFTY_TWO_WEEK_INDICATOR,
    RS80_INDICATOR,
    RS50_INDICATOR,
    RANGE_BREAKOUT_INDICATOR,
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
