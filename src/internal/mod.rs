/// 數據回補
pub mod backfill;
/// 聊天機器人
pub mod bot;
/// 數據快取
pub mod cache_share;
/// 計算類
pub mod calculation;
/// 抓取數據類
mod crawler;
/// 資料庫操作
mod database;
/// 提醒類
mod reminder;
/// 工作排程
pub mod scheduler;
/// 工具類
pub mod util;


/// 交易所
#[derive(Debug, Copy, Clone)]
pub enum StockExchange {
    /// 臺灣證券交易所 1
    TWSE,
    /// 證券櫃檯買賣市場 2
    TPEx,
}

impl StockExchange {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockExchange::TWSE => 1,
            StockExchange::TPEx => 2,
        }
    }

    pub fn iterator() -> impl Iterator<Item = Self> {
        [Self::TWSE, Self::TPEx]
            .iter()
            .copied()
    }
}


/// 市場別
#[derive(Debug, Copy, Clone)]
pub enum StockExchangeMarket {
    /// 上市 2
    Listed,
    /// 上櫃 4
    OverTheCounter,
    /// 興櫃 5
    Emerging,
}

impl StockExchangeMarket {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockExchangeMarket::Listed => 2,
            StockExchangeMarket::OverTheCounter => 4,
            StockExchangeMarket::Emerging => 5,
        }
    }

    pub fn exchange(&self) -> StockExchange {
        match self {
            StockExchangeMarket::Listed => StockExchange::TWSE,
            StockExchangeMarket::OverTheCounter => StockExchange::TPEx,
            StockExchangeMarket::Emerging => StockExchange::TPEx,
        }
    }

    pub fn iterator() -> impl Iterator<Item = Self> {
        [Self::Listed, Self::OverTheCounter, Self::Emerging]
            .iter()
            .copied()
    }
}