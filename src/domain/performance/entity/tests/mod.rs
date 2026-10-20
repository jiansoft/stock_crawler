//! 績效實體的測試。
//!
//! 子模組：`periods`（期間與指標代碼）、`records`（股利事件、覆蓋率與快取鍵）、`corporate_actions`
//! （公司行動參考價）；這裡放共用的日期建構函式。

mod corporate_actions;
mod periods;
mod records;

use super::*;
use crate::core::util::map::Keyable;

pub(super) fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}
