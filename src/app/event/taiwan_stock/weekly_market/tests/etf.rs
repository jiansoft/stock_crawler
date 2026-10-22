//! ETF 資金流向一段：流入／流出排序與數量上限、持有 ETF 的淨申購與折溢價、整則訊息的標題日期。

use std::collections::BTreeSet;

use rust_decimal_macros::dec;

use super::super::etf::{EtfWeekSection, etf_section, lines};
use super::*;
use crate::domain::etf::{
    EtfHoldingPeriod, EtfHoldingPeriods, EtfHoldingRecord, EtfWeeklyFlow,
    test_double::RecordingEtfRepository,
};

fn flow(symbol: &str, net_flow: f64) -> EtfWeeklyFlow {
    EtfWeeklyFlow {
        stock_symbol: symbol.to_owned(),
        name: format!("名稱{symbol}"),
        net_flow,
        premium_avg: Some(0.214),
        premium_last: None,
        from: day(19),
        to: day(23),
    }
}

#[tokio::test]
async fn section_keeps_held_etfs_with_data_and_lists_top_flows() {
    let mut flows: Vec<EtfWeeklyFlow> = (1..=7)
        .map(|i| flow(&format!("00{i}"), f64::from(i) * 1e8))
        .collect();
    flows.push(flow("0056", -3.25e8));
    flows.push(flow("00679B", 0.0));
    let repo = RecordingEtfRepository {
        weekly: flows,
        ..Default::default()
    };
    let holdings: BTreeSet<String> = ["0056", "2330", "00679B"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let section = etf_section(&repo, &holdings, day(24))
        .await
        .expect("查詢成功")
        .expect("有資料");
    assert_eq!((section.from, section.to), (day(19), day(23)));
    assert_eq!(section.held, vec!["0056".to_owned(), "00679B".to_owned()]);

    let text = lines(&section);
    assert_eq!(text[0], "");
    assert_eq!(text[1], "ETF 資金流向（10-19～10-23，受益單位增減 × 淨值）");
    assert_eq!(
        text[2],
        "淨流入：007 名稱007 +7.0 億、006 名稱006 +6.0 億、005 名稱005 +5.0 億、004 名稱004 +4.0 億、003 名稱003 +3.0 億"
    );
    assert_eq!(text[3], "淨流出：0056 名稱0056 -3.2 億");
    assert_eq!(
        text[4],
        "持有 0056 名稱0056：淨申購 -3.2 億，折溢價週平均 +0.21%（最新 無）"
    );
    assert_eq!(text.len(), 6, "淨申購為 0 的持有 ETF 仍列出");

    let empty = RecordingEtfRepository::default();
    assert!(
        etf_section(&empty, &holdings, day(24))
            .await
            .expect("查詢成功")
            .is_none()
    );
    let quiet = EtfWeekSection {
        from: day(19),
        to: day(23),
        flows: vec![flow("0050", 0.0)],
        held: Vec::new(),
        changes: Vec::new(),
    };
    assert!(
        lines(&quiet).is_empty(),
        "沒有流入流出也沒有持有時整段不出現"
    );
}

#[test]
fn etf_section_alone_makes_a_message_dated_by_its_last_day() {
    let section = EtfWeekSection {
        from: day(19),
        to: day(23),
        flows: vec![flow("0050", 2e8)],
        held: Vec::new(),
        changes: Vec::new(),
    };
    let message = build_message(
        None,
        None,
        None,
        &[],
        HoldingSection {
            etfs: Some(&section),
            ..Default::default()
        },
        None,
        |symbol: &str| symbol.to_owned(),
    )
    .expect("有 ETF 一段");
    assert!(
        message.starts_with(&esc("每週市場摘要︰2026-10-23\n")),
        "{message}"
    );
    assert!(
        message.contains(&esc("淨流入：0050 名稱0050 +2.0 億")),
        "{message}"
    );
}

fn holding(rank: i16, symbol: &str, name: &str, weight: rust_decimal::Decimal) -> EtfHoldingRecord {
    EtfHoldingRecord {
        rank,
        holding_symbol: symbol.to_owned(),
        holding_name: name.to_owned(),
        weight,
    }
}

#[tokio::test]
async fn held_etf_with_new_holding_period_lists_changes() {
    let period = |month, rows: Vec<EtfHoldingRecord>| EtfHoldingPeriod {
        as_of: NaiveDate::from_ymd_opt(2026, month, 1).expect("合法日期"),
        holdings: rows,
    };
    let repo = RecordingEtfRepository {
        weekly: vec![flow("0050", 1e8)],
        periods: vec![
            EtfHoldingPeriods {
                stock_symbol: "0050".to_owned(),
                previous: period(
                    9,
                    vec![
                        holding(1, "2330", "台積電", dec!(56.40)),
                        holding(2, "2317", "鴻海", dec!(4.80)),
                    ],
                ),
                current: period(
                    10,
                    vec![
                        holding(1, "2330", "台積電", dec!(58.10)),
                        holding(2, "2308", "台達電", dec!(2.30)),
                    ],
                ),
            },
            // 沒有差異的不列。
            EtfHoldingPeriods {
                stock_symbol: "0056".to_owned(),
                previous: period(9, vec![holding(1, "2454", "聯發科", dec!(4.00))]),
                current: period(10, vec![holding(1, "2454", "聯發科", dec!(4.00))]),
            },
        ],
        ..Default::default()
    };
    let holdings: BTreeSet<String> = ["0050", "0056"].iter().map(|s| s.to_string()).collect();
    let section = etf_section(&repo, &holdings, day(24))
        .await
        .expect("查詢成功")
        .expect("有資料");
    assert_eq!(section.changes.len(), 1);
    let text = lines(&section);
    assert_eq!(
        text.last().map(String::as_str),
        Some(concat!(
            "持有 0050 名稱0050 前十大持股更新（09-01 → 10-01）：新進 2308 台達電 2.30%；",
            "剔除 2317 鴻海 4.80%；權重變化 2330 台積電 56.40% → 58.10%（+1.70）"
        ))
    );

    // 只有持股換期、沒有任何淨申購時，整段仍出現。
    let quiet = EtfWeekSection {
        flows: vec![flow("0050", 0.0)],
        held: Vec::new(),
        ..section
    };
    assert!(
        lines(&quiet)
            .iter()
            .any(|line| line.contains("前十大持股更新"))
    );
}
