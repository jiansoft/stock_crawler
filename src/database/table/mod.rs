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
/// 指數數據表。
pub mod index;
/// 最新一日報價彙總表。
pub mod last_daily_quotes;
/// 月營收資料表。
pub mod revenue;
/// 股票主檔。
pub mod stock;
/// 股票交易所與市場別對照表。
pub mod stock_exchange_market;
mod stock_index;
/// 持股名細
pub mod stock_ownership_details;
mod stock_word;

/// 系統設定資料表。
pub mod config;
/// 每日市值記錄各
pub mod daily_money_history;
/// 每日市值記錄各檔股票的統計值
pub mod daily_money_history_detail;
/// 每日市值記錄各檔股票的股數明細
pub mod daily_money_history_detail_more;
/// 每日市值記錄各會員垂直總覽
pub mod daily_money_history_member;
/// 每日股票價格估值統計
pub mod daily_stock_price_stats;
/// 股票便宜、合理、昂貴價的估算
pub mod estimate;
/// 股票歷史最高、最低等數據
pub mod quote_history_record;
/// 追踪即時股價，當超過或低於設定的數值時發送TG訊息
pub mod trace;
/// 殖利率排行
pub mod yield_rank;
