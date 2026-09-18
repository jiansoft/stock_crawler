//! # ETF 基本資料載體
//!
//! 定義 TWSE 與 TPEx 採集 ETF 清單時共用的資料結構，
//! 讓上層流程不必分辨資料來自哪個市場。

use crate::core::declare::StockExchangeMarket;

/// 台灣 ETF 資訊載體。
///
/// 此結構用於存儲從 TWSE 或 TPEx 採集到的 ETF 基本資料。
#[derive(Debug, Clone)]
pub struct EtfInfo {
    /// 股票代號（例如："0050"）。
    pub stock_symbol: String,
    /// 股票名稱（例如："元大台灣50"）。
    pub name: String,
    /// 上市日期（格式：YYYY-MM-DD）。
    pub listing_date: String,
    /// 產業分類名稱（ETF 固定為 "ETF"）。
    pub industry: String,
    /// 交易市場。
    pub market: StockExchangeMarket,
    /// 產業分類 ID（專案中 ETF 的固定 ID 是 9001）。
    pub industry_id: i32,
}
