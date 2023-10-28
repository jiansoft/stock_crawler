/// 數據回補
pub mod backfill;
/// 聊天機器人
pub mod bot;
/// 數據快取
pub mod cache;
/// 計算類
pub mod calculation;
/// 設定檔
pub mod config;
/// 抓取數據類
mod crawler;
/// 資料庫操作
mod database;
/// 事件
pub mod event;
/// nosql
pub mod nosql;
///
pub mod rpc;
/// 工作排程
pub mod scheduler;

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
        [Self::TWSE, Self::TPEx].iter().copied()
    }
}

/// 市場別
#[derive(PartialEq, Debug, Copy, Clone)]
#[repr(i32)]
#[non_exhaustive]
pub enum StockExchangeMarket {
    /// 上市 2
    Listed = 2,
    /// 上櫃 4
    OverTheCounter = 4,
    /// 興櫃 5
    Emerging = 5,
}

impl StockExchangeMarket {
    pub fn serial_number(&self) -> i32 {
        *self as i32
    }

    pub fn from_serial_number(serial: i32) -> Option<StockExchangeMarket> {
        match serial {
            _ if serial == StockExchangeMarket::Listed as i32 => Some(StockExchangeMarket::Listed),
            _ if serial == StockExchangeMarket::OverTheCounter as i32 => {
                Some(StockExchangeMarket::OverTheCounter)
            }
            _ if serial == StockExchangeMarket::Emerging as i32 => {
                Some(StockExchangeMarket::Emerging)
            }
            _ => None,
        }
    }

    pub fn exchange(&self) -> StockExchange {
        match self {
            StockExchangeMarket::Listed => StockExchange::TWSE,
            StockExchangeMarket::OverTheCounter | StockExchangeMarket::Emerging => {
                StockExchange::TPEx
            }
        }
    }

    pub fn iterator() -> impl Iterator<Item = Self> {
        [Self::Listed, Self::OverTheCounter, Self::Emerging]
            .iter()
            .copied()
    }
}
