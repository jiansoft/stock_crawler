//! 每週市場摘要的區間突破一段。

use super::super::breakout::{BreakoutSection, breakout_section, lines};
use super::fakes::*;
use super::*;
use crate::domain::quote::technical::{RankedSignal, SignalOutcome};

fn ranked(symbol: &str, trade_value: f64) -> RankedSignal {
    RankedSignal {
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        industry: String::new(),
        trade_value,
        description: String::new(),
    }
}

fn outcome(samples: i64) -> SignalOutcome {
    SignalOutcome {
        indicator: "range_breakout".to_owned(),
        kind: "breakout".to_owned(),
        samples,
        avg_return: 0.84,
        win_rate: 0.48,
    }
}

/// 一週總檔次為每天加總；成交值前 3 檔跨日比較，同一檔只取一次；樣本足夠才附歷史表現。
#[tokio::test]
async fn section_sums_days_and_picks_top_by_trade_value() {
    let mut fake = quote();
    fake.breakouts = HashMap::from([
        (day(16), (vec![ranked("A", 300.0), ranked("B", 100.0)], 12)),
        (day(15), (vec![ranked("A", 500.0), ranked("C", 200.0)], 8)),
        (day(12), (vec![ranked("D", 50.0)], 1)),
    ]);
    fake.outcomes = vec![outcome(120)];

    let section = breakout_section(&fake, day(17))
        .await
        .expect("成功")
        .expect("有交易日");

    assert_eq!(
        (section.from, section.to, section.total),
        (day(12), day(16), 21)
    );
    let top: Vec<(&str, f64)> = section
        .top
        .iter()
        .map(|signal| (signal.stock_symbol.as_str(), signal.trade_value))
        .collect();
    assert_eq!(top, [("A", 500.0), ("C", 200.0), ("B", 100.0)]);
    assert_eq!(section.outcome, Some(outcome(120)));

    fake.outcomes = vec![outcome(10)];
    let small = breakout_section(&fake, day(17))
        .await
        .expect("成功")
        .expect("有交易日");
    assert_eq!(small.outcome, None);
}

/// 沒有交易日回 None；查詢失敗回錯誤。
#[tokio::test]
async fn section_without_dates_or_on_failure() {
    let empty = FakeQuote::default();
    assert_eq!(breakout_section(&empty, day(17)).await.expect("成功"), None);

    let mut failing = quote();
    failing.fail = true;
    assert!(breakout_section(&failing, day(17)).await.is_err());
}

/// 一週都沒有訊號時不輸出；有訊號時列總檔次、前幾檔與歷史表現，並出現在整則訊息裡。
#[test]
fn lines_and_message() {
    let mut section = BreakoutSection {
        from: day(12),
        to: day(16),
        total: 0,
        top: Vec::new(),
        outcome: None,
    };
    assert!(lines(&section).is_empty());

    section.total = 21;
    section.top = vec![ranked("2330", 500.0), ranked("2317", 300.0)];
    section.outcome = Some(outcome(120));
    assert_eq!(
        lines(&section),
        [
            String::new(),
            "區間突破（2026-10-12～2026-10-16）".to_owned(),
            "• 共 21 檔次；成交值前 2：2330 名2330、2317 名2317".to_owned(),
            "• 近一年同訊號 20 日勝率 48%、平均還原報酬 +0.8%（120 次）；回測單獨使用輸 0050"
                .to_owned(),
        ]
    );

    let message = build_message(
        None,
        None,
        None,
        &[],
        HoldingSection {
            breakouts: Some(&section),
            ..Default::default()
        },
        None,
        |_| String::new(),
    )
    .expect("有內容");
    assert!(
        message.contains(r"區間突破（2026\-10\-12～2026\-10\-16）"),
        "{message}"
    );
}
