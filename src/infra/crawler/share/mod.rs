//! # 爬蟲共用模組 (Share)
//!
//! 此模組收納多個採集站台共用的資料載體 (DTO)、抓取介面與解析邏輯，
//! 例如年度獲利、每日收盤報價、月營收、除權除息公告、外資持股與公網 IP 查詢。
//!
//! 內容依主題拆分成數個子檔案，各子檔案本身為私有模組，
//! 由此處統一以 `pub use` 重新匯出，讓既有的 `crawler::share::Xxx`
//! 呼叫路徑完全維持不變。

mod annual_profit;
mod daily_quote;
mod dividend;
mod etf;
mod public_ip;
mod qfii;
mod quote_field;
mod revenue;

pub(super) use annual_profit::fetch_annual_profits;
pub use annual_profit::{AnnualProfit, AnnualProfitFetcher};
pub use daily_quote::DailyQuoteDto;
pub use dividend::{ExDividendAnnouncement, parse_ex_dividend_kind};
pub use etf::EtfInfo;
pub use public_ip::get_public_ip;
pub use qfii::QfiiDto;
pub use quote_field::QuoteParseError;
pub(crate) use quote_field::{ensure_rejected_rows_within_threshold, parse_quote_decimal};
pub use revenue::RevenueDto;
