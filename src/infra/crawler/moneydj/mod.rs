/// MoneyDJ 年度獲利資料 crawler。
pub mod annual_profit;
/// 股利政策表（除權除息日程）
pub mod dividend_schedule;
/// ETF 歷史淨值與市價（回補 `etf_daily_nav`）。
pub mod etf_nav;

/// 理財網的資料查詢主機（年度獲利等 djjson 介面）。
const HOST: &str = "justdata.moneydj.com";

/// 理財網的網站主機（股利政策表等 djhtm 頁面）。
pub(crate) const WEB_HOST: &str = "moneydj.com";
