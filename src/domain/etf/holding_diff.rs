//! 前十大持股兩期之間的差異：新進、剔除與權重變化。
//!
//! 以成分股代號比對；債券等沒有代號的持股改用名稱比對。

use std::collections::HashMap;

use rust_decimal::prelude::ToPrimitive;

use super::EtfHoldingRecord;

/// 一檔持股與它的比例（%）。
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingWeight {
    /// 名稱（有代號時為「代號 名稱」）。
    pub label: String,
    /// 比例（%）。
    pub weight: f64,
}

/// 兩期都在前十大、比例有變的持股。
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingMove {
    /// 名稱（有代號時為「代號 名稱」）。
    pub label: String,
    /// 前一期比例（%）。
    pub before: f64,
    /// 這一期比例（%）。
    pub after: f64,
}

/// 兩期前十大持股的差異。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HoldingDiff {
    /// 這一期新進前十大，依比例由大到小。
    pub added: Vec<HoldingWeight>,
    /// 這一期掉出前十大，依前一期比例由大到小。
    pub removed: Vec<HoldingWeight>,
    /// 兩期都在、比例有變，依變化幅度由大到小。
    pub moved: Vec<HoldingMove>,
}

impl HoldingDiff {
    /// 沒有任何差異。
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.moved.is_empty()
    }
}

/// 比例變化小於這個值（百分點）視為沒變，避免四捨五入的雜訊。
const MIN_MOVE: f64 = 0.01;

fn key(record: &EtfHoldingRecord) -> &str {
    if record.holding_symbol.is_empty() {
        &record.holding_name
    } else {
        &record.holding_symbol
    }
}

fn label(record: &EtfHoldingRecord) -> String {
    if record.holding_symbol.is_empty() {
        record.holding_name.clone()
    } else {
        format!("{} {}", record.holding_symbol, record.holding_name)
    }
}

fn weight(record: &EtfHoldingRecord) -> f64 {
    record.weight.to_f64().unwrap_or_default()
}

/// 比較前一期與這一期的前十大持股。
pub fn diff_holdings(previous: &[EtfHoldingRecord], current: &[EtfHoldingRecord]) -> HoldingDiff {
    let before: HashMap<&str, &EtfHoldingRecord> = previous.iter().map(|r| (key(r), r)).collect();
    let after: HashMap<&str, &EtfHoldingRecord> = current.iter().map(|r| (key(r), r)).collect();
    let pick = |rows: &[EtfHoldingRecord], other: &HashMap<&str, &EtfHoldingRecord>| {
        let mut out: Vec<HoldingWeight> = rows
            .iter()
            .filter(|r| !other.contains_key(key(r)))
            .map(|r| HoldingWeight {
                label: label(r),
                weight: weight(r),
            })
            .collect();
        out.sort_by(|a, b| b.weight.total_cmp(&a.weight));
        out
    };
    let mut moved: Vec<HoldingMove> = current
        .iter()
        .filter_map(|r| {
            let old = weight(before.get(key(r))?);
            let new = weight(r);
            ((new - old).abs() >= MIN_MOVE).then(|| HoldingMove {
                label: label(r),
                before: old,
                after: new,
            })
        })
        .collect();
    moved.sort_by(|a, b| {
        (b.after - b.before)
            .abs()
            .total_cmp(&(a.after - a.before).abs())
    });
    HoldingDiff {
        added: pick(current, &before),
        removed: pick(previous, &after),
        moved,
    }
}

#[cfg(test)]
mod tests {
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
}
