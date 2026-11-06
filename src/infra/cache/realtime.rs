//! 即時報價快照型別。
//!
//! 此模組只負責描述「單一股票在某個時間點的即時報價狀態」，
//! 不包含抓取、更新或快取生命週期控制邏輯。

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::core::declare;

/// 當日漲跌幅限制，來自 Yahoo 類股報價的 `limitUpPrice`／`limitDownPrice`，
/// 以及開盤前載入的交易所漲跌停價（`app::event::trace::price_limits`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PriceLimit {
    /// 來源沒有提供，沿用固定門檻判斷。
    #[default]
    Unknown,
    /// 沒有漲跌幅限制（國外成分 ETF，如 00715L、00646）。
    Unlimited,
    /// 當日的跌停價與漲停價。
    Range {
        /// 跌停價。
        down: Decimal,
        /// 漲停價。
        up: Decimal,
    },
}

impl PriceLimit {
    /// 交易所表示「無漲跌幅限制」的跌停價（漲停價同時是 9,999.95）。
    const EXCHANGE_UNLIMITED_DOWN: Decimal = Decimal::from_parts(1, 0, 0, false, 2);
    /// 漲停價至少這麼高才視為交易所的「無漲跌幅限制」寫法。
    const EXCHANGE_UNLIMITED_UP: Decimal = Decimal::from_parts(9_999, 0, 0, false, 0);

    /// 由交易所公告的跌停價與漲停價建立限制：跌停 0.01 且漲停 9,999.95 是「無漲跌幅限制」，
    /// 兩者皆為正且跌停不高於漲停是區間，其餘視為未知。
    pub fn from_exchange_prices(down: Decimal, up: Decimal) -> Self {
        if down <= Self::EXCHANGE_UNLIMITED_DOWN && up >= Self::EXCHANGE_UNLIMITED_UP {
            return Self::Unlimited;
        }
        if down > Decimal::ZERO && down <= up {
            Self::Range { down, up }
        } else {
            Self::Unknown
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeSnapshot {
    /// 股票代號 (必要欄位)
    pub symbol: String,
    /// 股票名稱
    pub name: String,
    /// 本次報價資料的採集站點
    pub source_site: String,
    /// 成交價 (必要欄位)
    pub price: Decimal,
    /// 漲跌
    pub change: Decimal,
    /// 漲跌幅 (%)
    pub change_range: Decimal,
    /// 開盤價
    pub open: Decimal,
    /// 最高價
    pub high: Decimal,
    /// 最低價
    pub low: Decimal,
    /// 昨收價
    pub last_close: Decimal,
    /// 成交量 (單位：張)
    pub volume: Decimal,
    /// 快照最後寫入快取的 UTC 時間，供外部 API 判斷資料新鮮度。
    pub updated_at: DateTime<Utc>,
    /// 當日漲跌幅限制；只有 Yahoo 類股報價會提供。
    pub price_limit: PriceLimit,
}

impl RealtimeSnapshot {
    /// 建立新的報價快照，強制要求填入必要欄位。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    /// - `price`: 最新成交價。
    ///
    /// # 回傳
    /// - `RealtimeSnapshot`: 其餘欄位以零值或空字串初始化，
    ///   方便後續由解析流程逐步補齊。
    pub fn new(symbol: String, price: Decimal) -> Self {
        Self {
            symbol,
            price,
            name: String::new(),
            source_site: String::new(),
            change: Decimal::ZERO,
            change_range: Decimal::ZERO,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            last_close: Decimal::ZERO,
            volume: Decimal::ZERO,
            updated_at: Utc::now(),
            price_limit: PriceLimit::Unknown,
        }
    }

    /// 將快照轉換為外部介面使用的報價型別 `declare::StockQuotes`。
    ///
    /// 由於外部介面使用 `f64`，這一步會進行型別轉換，
    /// 若有無法轉換的值會傳回 `Err` 並標明失敗欄位。
    pub fn try_into_stock_quotes(&self) -> Result<declare::StockQuotes> {
        Ok(declare::StockQuotes {
            stock_symbol: self.symbol.clone(),
            price: self
                .price
                .to_f64()
                .context("Decimal to f64 conversion failed (price)")?,
            change: self
                .change
                .to_f64()
                .context("Decimal to f64 conversion failed (change)")?,
            change_range: self
                .change_range
                .to_f64()
                .context("Decimal to f64 conversion failed (range)")?,
        })
    }
}
