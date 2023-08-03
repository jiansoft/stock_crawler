/// 每日股票報價數據
pub mod daily_quote;
/// 年度股利發放明細與總計
pub mod dividend;
/// 持股股息發放記錄表(總計)
pub mod dividend_record_detail;
/// 持股股息發放明細記錄表
pub mod dividend_record_detail_more;
/// 公司每季獲利能力
pub mod financial_statement;
pub mod index;
pub mod last_daily_quotes;
pub mod revenue;
pub mod stock;
mod stock_index;
/// 持股名細
pub mod stock_ownership_details;
mod stock_word;
// 股票交易所的市場
pub mod stock_exchange_market;

pub mod config;
/// 股票歷史最高、最低等數據
pub mod quote_history_record;