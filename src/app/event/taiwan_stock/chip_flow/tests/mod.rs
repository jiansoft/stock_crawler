//! 持股三大法人與融資融券通知的測試。
//!
//! 子模組：`reasons`（通知理由判斷）、`message`（訊息格式）、`streak_store`（Redis 存取）；這裡放共用的日期、
//! 法人買賣與報告建構函式。

mod message;
mod reasons;
mod streak_store;

use super::*;

pub(super) fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

pub(super) fn flow(foreign: i64, trust: i64, dealer: i64) -> InstitutionalFlow {
    InstitutionalFlow {
        foreign,
        trust,
        dealer,
    }
}

pub(super) fn report(
    flow: Option<InstitutionalFlow>,
    margin: Option<MarginBalance>,
    volume: Option<i64>,
    streak: Streak,
) -> ChipReport {
    ChipReport {
        stock_symbol: "2330".to_string(),
        flow,
        margin,
        volume,
        streak,
    }
}
