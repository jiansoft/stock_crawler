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
/// 股票便宜、合理、昂貴價的估算
pub mod estimate;
/// 殖利率排行
pub mod yield_rank ;
/// 每日市值記錄各
pub mod daily_money_history;
/// 每日市值記錄各檔股票的統計值
pub mod daily_money_history_detail;
/// 每日市值記錄各檔股票的股數明細
pub mod daily_money_history_detail_more;