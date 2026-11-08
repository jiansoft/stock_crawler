//! `/stocks/{symbol}/dividend-fill` handler：最近幾次除權息的填息天數的測試。

use rust_decimal_macros::dec;

use super::*;

fn row(ex_date: &str) -> ExRightEvent {
    ExRightEvent {
        ex_date: NaiveDate::parse_from_str(ex_date, "%Y-%m-%d").expect("測試日期合法"),
        cash_dividend: dec!(1),
        stock_dividend: Decimal::ZERO,
    }
}

/// 最新收盤日之後（尚未除息）的事件略過，其餘由新到舊取前 limit 筆；沒有日線時為空。
#[test]
fn select_events_filters_and_limits() {
    let latest = NaiveDate::from_ymd_opt(2026, 10, 8);
    let rows = || {
        vec![
            row("2025-06-12"),
            row("2026-12-10"),
            row("2026-09-16"),
            row("2026-06-11"),
        ]
    };
    let dates: Vec<String> = select_events(rows(), latest, 2)
        .iter()
        .map(|e| e.ex_date.to_string())
        .collect();
    assert_eq!(dates, ["2026-09-16", "2026-06-11"]);
    assert_eq!(select_events(rows(), latest, 40).len(), 3);
    assert!(select_events(rows(), None, 40).is_empty());
}
