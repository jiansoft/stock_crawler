//! 即時報價快照型別。
//!
//! 此模組只負責描述「單一股票在某個時間點的即時報價狀態」，
//! 不包含抓取、更新或快取生命週期控制邏輯。

use anyhow::{Context, Result};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::declare;

/// 即時報價快照。
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
