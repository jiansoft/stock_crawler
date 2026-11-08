//! `/etfs/{symbol}/holdings/history` handler：ETF 近幾期的前十大持股與期間差異的測試。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::*;
use crate::domain::etf::EtfHoldingRecord;

fn record(rank: i16, symbol: &str, name: &str, weight: &str) -> EtfHoldingRecord {
    EtfHoldingRecord {
        rank,
        holding_symbol: symbol.to_owned(),
        holding_name: name.to_owned(),
        weight: weight.parse::<Decimal>().expect("比例合法"),
    }
}

fn period(month: u32, holdings: Vec<EtfHoldingRecord>) -> EtfHoldingPeriod {
    EtfHoldingPeriod {
        as_of: NaiveDate::from_ymd_opt(2026, month, 1).expect("日期合法"),
        holdings,
    }
}

#[test]
fn each_period_lists_changes_against_the_one_before() {
    let history = EtfHoldingHistory {
        stock_symbol: "0056".to_owned(),
        name: "元大高股息".to_owned(),
        periods: vec![
            period(
                10,
                vec![
                    record(1, "2454", "聯發科", "8.456"),
                    record(2, "2303", "聯電", "4.0"),
                ],
            ),
            period(
                9,
                vec![
                    record(1, "2454", "聯發科", "7.5"),
                    record(2, "2412", "中華電", "3.2"),
                ],
            ),
            // 多取的一期：只用來算 9 月的差異，不回傳。
            period(8, vec![record(1, "2454", "聯發科", "7.5")]),
        ],
    };
    let response = history_response(history, 2);
    assert_eq!(response.periods.len(), 2);
    let latest = &response.periods[0];
    assert_eq!(latest.as_of, "2026-10-01");
    assert_eq!(latest.holdings[0].weight, 8.46);
    let changes = latest.changes.as_ref().expect("有前一期");
    assert_eq!(changes.previous_as_of, "2026-09-01");
    assert_eq!(changes.added[0].label, "2303 聯電");
    assert_eq!(changes.removed[0].label, "2412 中華電");
    assert_eq!(
        (changes.moved[0].before, changes.moved[0].after),
        (7.5, 8.46)
    );
    let september = response.periods[1].changes.as_ref().expect("有 8 月");
    assert_eq!(september.added[0].label, "2412 中華電");
    assert!(september.moved.is_empty() && september.removed.is_empty());
}

#[test]
fn the_oldest_period_without_history_has_no_changes() {
    let history = EtfHoldingHistory {
        periods: vec![period(10, vec![record(1, "", "US TREASURY 2055", "5")])],
        ..Default::default()
    };
    let response = history_response(history, 6);
    assert_eq!(response.periods.len(), 1);
    assert_eq!(response.periods[0].holdings[0].symbol, "");
    assert!(response.periods[0].changes.is_none());
    assert!(
        history_response(EtfHoldingHistory::default(), 6)
            .periods
            .is_empty()
    );
}
