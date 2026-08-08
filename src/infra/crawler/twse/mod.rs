/// 台股財報
pub mod eps;
/// ETF 資訊
pub mod etf;
/// 台股休市日期
pub mod holiday_schedule;
/// 國際證券辨識
pub mod international_securities_identification_number;
/// 公開申購公告-抽籤日程表
pub mod public;
/// 外資及陸資投資持股
pub mod qualified_foreign_institutional_investor;
/// 台股收盤報價-上市
pub mod quote;
/// 月營收
pub mod revenue;
/// 個股日成交資訊（單一股票、整個月）
pub mod stock_day;
/// 終止上市公司
pub mod suspend_listing;
/// 台股加權指數
pub mod taiwan_capitalization_weighted_stock_index;

pub const HOST: &str = "twse.com.tw";
