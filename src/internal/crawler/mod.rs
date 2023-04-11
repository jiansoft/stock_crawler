/// dds
pub mod free_dns;
/// 國際證券識別碼
pub mod international_securities_identification_number;
/// 台股每日收盤後股票的報價
pub mod quotes;
/// 每月營收
pub mod revenue;
/// 台股指數
pub mod taiwan_capitalization_weighted_stock_index;
/// 雅虎財經
pub mod yahoo;
/// 台灣證卷交易所
pub mod twse;

/// 市場別
#[derive(Debug, Copy, Clone)]
pub enum StockMarket {
    /// 上市
    Listed,
    /// 上櫃
    OverTheCounter,
    /// 興櫃
    Emerging,
}

impl StockMarket {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockMarket::Listed => 2,
            StockMarket::OverTheCounter => 4,
            StockMarket::Emerging => 5,
        }
    }
    pub fn iterator() -> impl Iterator<Item = Self> {
        [Self::Listed, Self::OverTheCounter, Self::Emerging]
            .iter()
            .copied()
    }
}
