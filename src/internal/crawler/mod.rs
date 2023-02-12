/// 國際證券識別碼
pub mod international_securities_identification_number;
/// 台股指數
pub mod taiwan_capitalization_weighted_stock_index;
/// 已下市股票
pub mod suspend_listing;


/// 市場別
pub enum StockMarket {
    /// 上市
    StockExchange,
    /// 上櫃
    OverTheCounter,
}

impl StockMarket {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockMarket::StockExchange => 2,
            StockMarket::OverTheCounter => 4,
        }
    }
}