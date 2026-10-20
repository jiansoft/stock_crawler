//! 持股籌碼日報：某交易日收盤時仍持有的普通股，當天的三大法人買賣超、融資融券增減與主力進出，
//! 並依 Telegram 通知（持股法人與信用交易、持股主力進出）的門檻標出達標原因。
//!
//! 門檻集中在這裡，`/portfolio/chip` 與兩則通知共用；stock_go 的持股籌碼頁也照抄同一組數字，
//! 調整時要一起改。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::domain::chip::BrokerNetRecord;

/// 外資買賣超佔當日成交量的門檻（%）。
///
/// 門檻以 2026-10-05 的持股試算：外資 10%／100 張時 46 檔中有 24 檔達標（外資買賣超佔量
/// 本來就常在一到三成），拉高到 30%／1,000 張並配合下列門檻後剩 9 檔。
pub const FOREIGN_MIN_PERCENT: i64 = 30;
/// 外資買賣超的張數門檻。
pub const FOREIGN_MIN_LOTS: i64 = 1_000;
/// 投信買賣超佔當日成交量的門檻（%）；投信部位較小，門檻低於外資。
pub const TRUST_MIN_PERCENT: i64 = 15;
/// 投信買賣超的張數門檻。
pub const TRUST_MIN_LOTS: i64 = 200;
/// 外資連續同向買賣超的天數門檻。
pub const FOREIGN_STREAK_MIN: i32 = 5;
/// 投信連續同向買賣超的天數門檻。
pub const TRUST_STREAK_MIN: i32 = 3;
/// 融資餘額增減的比例門檻（%）。
pub const MARGIN_MIN_PERCENT: i64 = 10;
/// 融資餘額增減的張數門檻。
pub const MARGIN_MIN_LOTS: i64 = 500;
/// 融券餘額增減的比例門檻（%）。
pub const SHORT_MIN_PERCENT: i64 = 50;
/// 融券餘額增減的張數門檻。
pub const SHORT_MIN_LOTS: i64 = 200;
/// 主力買賣超佔成交量比重（%）的門檻。
///
/// 2026-10-05 持股 44 檔普通股中約三分之一超過 20%，再低會讓每天的訊息太長。
pub const BROKER_MIN_SHARE: Decimal = dec!(20);
/// 主力買賣超張數的門檻；成交清淡的股票幾十張就能有很高的比重，不具意義。
pub const BROKER_MIN_LOTS: i64 = 100;

/// 法人買賣超（股）是否同時達到佔成交量（股）比例與張數門檻；沒有成交量時不成立。
pub fn heavy_flow(net: i64, volume: Option<i64>, min_percent: i64, min_lots: i64) -> bool {
    volume.is_some_and(|volume| {
        volume > 0 && net.abs() * 100 >= volume * min_percent && net.abs() >= min_lots * 1_000
    })
}

/// 餘額（張）增減是否同時達到比例與張數門檻；前日為 0 時只看張數。
pub fn balance_changed(previous: i64, today: i64, min_percent: i64, min_lots: i64) -> bool {
    let delta = (today - previous).abs();
    delta >= min_lots && (previous == 0 || delta * 100 >= previous * min_percent)
}

/// 主力動作是否明顯：佔成交量比重與張數都達門檻。
pub fn broker_notable(main_share: Decimal, main_net: i64) -> bool {
    main_share.abs() >= BROKER_MIN_SHARE && main_net.abs() >= BROKER_MIN_LOTS
}

/// 達到通知門檻的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipReason {
    /// 外資買賣超佔成交量與張數都達門檻。
    ForeignHeavy,
    /// 投信買賣超佔成交量與張數都達門檻。
    TrustHeavy,
    /// 外資連續同向買賣超達門檻天數。
    ForeignStreak,
    /// 投信連續同向買賣超達門檻天數。
    TrustStreak,
    /// 融資餘額增減達門檻。
    Margin,
    /// 融券餘額增減達門檻。
    Short,
    /// 主力買賣超達門檻。
    Broker,
}

impl ChipReason {
    /// 機器可讀的代碼。
    pub fn code(self) -> &'static str {
        match self {
            Self::ForeignHeavy => "foreign_heavy",
            Self::TrustHeavy => "trust_heavy",
            Self::ForeignStreak => "foreign_streak",
            Self::TrustStreak => "trust_streak",
            Self::Margin => "margin",
            Self::Short => "short",
            Self::Broker => "broker",
        }
    }

    /// 與 Telegram 通知、stock_go 持股籌碼頁相同的中文標籤。
    pub fn label(self) -> &'static str {
        match self {
            Self::ForeignHeavy => "外資大量",
            Self::TrustHeavy => "投信大量",
            Self::ForeignStreak => "外資連續",
            Self::TrustStreak => "投信連續",
            Self::Margin => "融資",
            Self::Short => "融券",
            Self::Broker => "主力",
        }
    }
}

/// 某交易日一檔持股的原始籌碼；來源當天沒有該類資料時為 `None`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingChip {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱；查不到為空字串。
    pub name: String,
    /// 收盤價。
    pub closing_price: Option<Decimal>,
    /// 成交量（股）。
    pub volume: Option<i64>,
    /// 外資買賣超（股）。
    pub foreign_net: Option<i64>,
    /// 投信買賣超（股）。
    pub trust_net: Option<i64>,
    /// 自營商買賣超（股）。
    pub dealer_net: Option<i64>,
    /// 外資連續買賣超天數（正連買、負連賣）。
    pub foreign_streak: Option<i32>,
    /// 投信連續買賣超天數。
    pub trust_streak: Option<i32>,
    /// 前日融資餘額（張）。
    pub margin_previous: Option<i64>,
    /// 今日融資餘額（張）。
    pub margin_balance: Option<i64>,
    /// 前日融券餘額（張）。
    pub short_previous: Option<i64>,
    /// 今日融券餘額（張）。
    pub short_balance: Option<i64>,
    /// 主力進出；當天沒有抓到（非持股或 2026-10-07 以前）為 `None`。
    pub broker: Option<HoldingBroker>,
}

/// 持股當天的主力進出（張）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingBroker {
    /// 合計買超張數。
    pub buy_total: i64,
    /// 合計賣超張數。
    pub sell_total: i64,
    /// 主力買賣超佔成交量比重（%），正值為買超。
    pub main_share: Decimal,
    /// 買超前幾名分點。
    pub buyers: Vec<BrokerNetRecord>,
    /// 賣超前幾名分點。
    pub sellers: Vec<BrokerNetRecord>,
}

impl HoldingBroker {
    /// 主力買賣超張數（買超為正）。
    pub fn main_net(&self) -> i64 {
        self.buy_total - self.sell_total
    }
}

impl HoldingChip {
    /// 依 Telegram 通知的門檻列出達標原因；順序固定。
    pub fn reasons(&self) -> Vec<ChipReason> {
        let mut reasons = Vec::new();
        if self
            .foreign_net
            .is_some_and(|net| heavy_flow(net, self.volume, FOREIGN_MIN_PERCENT, FOREIGN_MIN_LOTS))
        {
            reasons.push(ChipReason::ForeignHeavy);
        }
        if self
            .trust_net
            .is_some_and(|net| heavy_flow(net, self.volume, TRUST_MIN_PERCENT, TRUST_MIN_LOTS))
        {
            reasons.push(ChipReason::TrustHeavy);
        }
        if self
            .foreign_streak
            .is_some_and(|days| days.abs() >= FOREIGN_STREAK_MIN)
        {
            reasons.push(ChipReason::ForeignStreak);
        }
        if self
            .trust_streak
            .is_some_and(|days| days.abs() >= TRUST_STREAK_MIN)
        {
            reasons.push(ChipReason::TrustStreak);
        }
        if let (Some(previous), Some(today)) = (self.margin_previous, self.margin_balance)
            && balance_changed(previous, today, MARGIN_MIN_PERCENT, MARGIN_MIN_LOTS)
        {
            reasons.push(ChipReason::Margin);
        }
        if let (Some(previous), Some(today)) = (self.short_previous, self.short_balance)
            && balance_changed(previous, today, SHORT_MIN_PERCENT, SHORT_MIN_LOTS)
        {
            reasons.push(ChipReason::Short);
        }
        if let Some(broker) = &self.broker
            && broker_notable(broker.main_share, broker.main_net())
        {
            reasons.push(ChipReason::Broker);
        }
        reasons
    }
}

/// 查詢日對應的交易日（不晚於查詢日、最近一個有籌碼資料的日子）與前後一個交易日。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipTradingDay {
    /// 交易日。
    pub date: NaiveDate,
    /// 前一個有籌碼資料的交易日。
    pub previous: Option<NaiveDate>,
    /// 後一個有籌碼資料的交易日。
    pub next: Option<NaiveDate>,
}

/// 持股籌碼日報的讀取介面。
#[async_trait]
pub trait HoldingChipQuery: Send + Sync {
    /// 不晚於 `date`（`None` 為今天）最近一個有籌碼資料的交易日；完全沒有資料回 `None`。
    async fn resolve_chip_day(&self, date: Option<NaiveDate>) -> Result<Option<ChipTradingDay>>;

    /// `date` 收盤時仍持有的普通股（不含 ETF）與當天籌碼，依代號排序。
    ///
    /// 持股批次的買進日不晚於當天，且未賣出、或賣出日（同會員同代號買進日之後最早一筆未撤銷的
    /// 賣出紀錄）晚於當天；找不到賣出紀錄的已賣出批次視為買進當天就不再持有。
    async fn fetch_holding_chips(&self, date: NaiveDate) -> Result<Vec<HoldingChip>>;
}

#[cfg(test)]
mod tests;
