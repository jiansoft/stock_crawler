/// 上櫃股票減資恢復買賣參考價格
pub mod capital_reduction;
/// 三大法人買賣超與融資融券餘額
pub mod chip;
/// ETF 資訊
pub mod etf;
/// 上櫃除權除息預告表
pub mod ex_dividend_announcement;
/// 上櫃除權除息計算結果表（已除權息的實際日期、現金股利與配股）
pub mod ex_right_result;
/// 興櫃每股淨值
pub mod net_asset_value_per_share;
/// 台股收盤報價-上櫃
pub(crate) mod quote;
/// 上櫃個股日成交資訊（單一證券的整月日報價）
pub mod stock_day;

pub const HOST: &str = "www.tpex.org.tw";
