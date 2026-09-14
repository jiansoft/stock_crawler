/// ETF 資訊
pub mod etf;
/// 上櫃除權除息預告表
pub mod ex_dividend_announcement;
/// 興櫃每股淨值
pub mod net_asset_value_per_share;
/// 台股收盤報價-上櫃
pub(crate) mod quote;

pub const HOST: &str = "www.tpex.org.tw";
