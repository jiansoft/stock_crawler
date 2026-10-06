//! 公開資訊觀測站（MOPS）/ 財務比較 E 點通。
//!
//! 目前此模組提供年度財報採集器，但尚未接入正式補齊流程。

pub mod annual_profit;
/// 上市公司股利分派情形
pub mod dividend_allotment;
/// 董監事持股餘額明細（含設質）
pub mod insider_holding;
/// 上市櫃公司每日重大訊息
pub mod material_news;

/// MOPS 財務比較 E 點通主機。
pub const HOST: &str = "mopsfin.twse.com.tw";
