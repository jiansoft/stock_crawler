//! `/portfolio/history` handler：持股的時間加權報酬、月報酬、最大回撤與大盤比較的測試。

use rust_decimal_macros::dec;

use super::*;

fn day(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, day).expect("有效日期")
}

/// 週末的買賣併入下一個交易日；同一交易日的多筆相加；超過最後一天的不計。
#[test]
fn attach_flows_moves_weekend_flows_to_next_trading_day() {
    let values = [day(24), day(27), day(28)]
        .into_iter()
        .map(|date| ValueRow {
            date,
            market_value: dec!(100),
        })
        .collect();
    let flows = vec![
        FlowRow {
            date: day(25),
            buys: dec!(10),
            sells: dec!(0),
        },
        FlowRow {
            date: day(27),
            buys: dec!(5),
            sells: dec!(3),
        },
        FlowRow {
            date: day(30),
            buys: dec!(99),
            sells: dec!(0),
        },
    ];
    let points = attach_flows(values, flows);
    assert_eq!(
        points
            .iter()
            .map(|point| (point.buys, point.sells))
            .collect::<Vec<_>>(),
        vec![(dec!(0), dec!(0)), (dec!(15), dec!(3)), (dec!(0), dec!(0))]
    );
}
