/// 年度股利發放明細與總計
pub mod dividend;
/// 財報、估值與營收資料表。
pub mod financial;
/// 指數數據表。
pub mod index;
/// 資金流向與會員市值資料表。
pub mod money_flow;
/// 報價、歷史報價與統計資料表。
pub mod quote;
/// 股票主檔。
pub mod stock;

/// 系統設定資料表。
pub mod config;
/// 追踪即時股價，當超過或低於設定的數值時發送TG訊息
pub mod trace;
/// 殖利率排行
pub mod yield_rank;

pub use dividend::{dividend_record_detail, dividend_record_detail_more};
pub use financial::{estimate, financial_statement, revenue};
pub use money_flow::{
    daily_money_history, daily_money_history_detail, daily_money_history_detail_more,
    daily_money_history_member,
};
pub use quote::{daily_quote, daily_stock_price_stats, last_daily_quotes, quote_history_record};
pub use stock::{stock_exchange_market, stock_ownership_details};
