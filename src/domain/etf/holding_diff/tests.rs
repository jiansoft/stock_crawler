//! 前十大持股兩期之間的差異：新進、剔除與權重變化的測試。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;

fn record(rank: i16, symbol: &str, name: &str, weight: Decimal) -> EtfHoldingRecord {
    EtfHoldingRecord {
        rank,
        holding_symbol: symbol.to_owned(),
        holding_name: name.to_owned(),
        weight,
    }
}

#[test]
fn finds_added_removed_and_largest_moves() {
    let previous = [
        record(1, "2330", "台積電", dec!(56.40)),
        record(2, "2317", "鴻海", dec!(4.80)),
        record(3, "2454", "聯發科", dec!(4.50)),
        record(4, "", "US TREASURY 2055", dec!(1.20)),
    ];
    let current = [
        record(1, "2330", "台積電", dec!(58.10)),
        record(2, "2454", "聯發科", dec!(4.20)),
        record(3, "2317", "鴻海", dec!(4.80)),
        record(4, "2308", "台達電", dec!(2.30)),
    ];
    let diff = diff_holdings(&previous, &current);
    assert_eq!(
        diff.added,
        vec![HoldingWeight {
            label: "2308 台達電".to_owned(),
            weight: 2.3
        }]
    );
    assert_eq!(
        diff.removed,
        vec![HoldingWeight {
            label: "US TREASURY 2055".to_owned(),
            weight: 1.2
        }]
    );
    let moves: Vec<&str> = diff.moved.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(moves, vec!["2330 台積電", "2454 聯發科"], "鴻海沒變不列");
    assert!(!diff.is_empty());
    assert!(diff_holdings(&current, &current).is_empty());
}
