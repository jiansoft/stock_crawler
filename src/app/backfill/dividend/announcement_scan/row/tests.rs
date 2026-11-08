//! 既有資料列的比對與資料列組裝的測試。

use rust_decimal_macros::dec;

use super::super::fixtures::{announcement, existing_row};
use super::*;

/// 公司改期時要以公告為準覆寫既有的除權息日。
#[test]
fn test_merge_announcement_dates_overwrites_changed_ex_date() {
    let mut row = existing_row(7, "2882", 2026, "", "2026-08-01", "2026-08-30");
    let ann = announcement("2882", (2026, 8, 15), true, false, Some(dec!(3.5)), None);

    assert!(merge_announcement_dates(&mut row, &ann, "2026-08-15", None));
    assert_eq!(row.ex_dividend_date_cash, "2026-08-15");
    // MoneyDJ 沒給發放日時保留資料庫原值，不可洗成未公布。
    assert_eq!(row.payable_date_cash, "2026-08-30");
}
