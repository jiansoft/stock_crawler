//! 實盤追蹤的單元測試。

use crate::domain::strategy::rebalance::RecordedHolding;

use super::*;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("固定日期合法")
}

fn bar(m: u32, d: u32, close: f64) -> PriceBar {
    PriceBar {
        date: day(m, d),
        close,
        high: close,
        low: close,
    }
}

fn holding(symbol: &str, weight: f64) -> RecordedHolding {
    RecordedHolding {
        symbol: symbol.to_owned(),
        name: None,
        weight,
    }
}

fn record(date: NaiveDate, holdings: Vec<RecordedHolding>, cash: f64) -> StrategyRebalanceRecord {
    StrategyRebalanceRecord {
        preset_name: "最佳組合".to_owned(),
        date,
        query: "from=2019-01-02".to_owned(),
        hold_days: 20,
        etf_weight: 50,
        cash,
        market_filtered: false,
        holdings,
    }
}

/// A 09-01 100 → 09-29 110 → 10-20 121；B 09-01 50 → 09-29 45；C 09-29 20 → 10-20 22；
/// 0050 100 → 102 → 105。
fn bars() -> HashMap<String, Vec<PriceBar>> {
    [
        (
            "A",
            vec![bar(9, 1, 100.0), bar(9, 29, 110.0), bar(10, 20, 121.0)],
        ),
        ("B", vec![bar(9, 1, 50.0), bar(9, 29, 45.0)]),
        ("C", vec![bar(9, 29, 20.0), bar(10, 20, 22.0)]),
        (
            "0050",
            vec![bar(9, 1, 100.0), bar(9, 29, 102.0), bar(10, 20, 105.0)],
        ),
    ]
    .into_iter()
    .map(|(symbol, bars)| (symbol.to_owned(), bars))
    .collect()
}

/// 每期持有到下次換股日、最後一期到最新交易日；策略部位依權重加總、現金 0，與 0050 依比例混合後連乘。
#[test]
fn track_chains_periods_and_compares_with_etf() {
    let records = [
        record(
            day(9, 1),
            vec![holding("A", 60.0), holding("B", 30.0)],
            10.0,
        ),
        record(
            day(9, 29),
            vec![holding("A", 50.0), holding("C", 50.0)],
            0.0,
        ),
    ];
    let response = track(
        "最佳組合",
        &records,
        Some(day(10, 20)),
        &bars(),
        &HashMap::new(),
    );
    assert_eq!(response.from.as_deref(), Some("2026-09-01"));
    assert_eq!(response.to.as_deref(), Some("2026-10-20"));
    let first = &response.periods[0];
    assert_eq!((first.end.as_str(), first.ongoing), ("2026-09-29", false));
    assert_eq!(
        first
            .holdings
            .iter()
            .map(|holding| holding.return_percent)
            .collect::<Vec<_>>(),
        [Some(10.0), Some(-10.0)]
    );
    // (60 × 10% + 30 × −10%) ÷ 100 = 3%；0050 +2%；各半 = 2.5%。
    assert_eq!(first.strategy_return, Some(3.0));
    assert_eq!(first.etf_return, Some(2.0));
    assert_eq!(first.portfolio_return, Some(2.5));
    let second = &response.periods[1];
    assert_eq!((second.end.as_str(), second.ongoing), ("2026-10-20", true));
    assert_eq!(second.strategy_return, Some(10.0));
    assert_eq!(second.etf_return, Some(2.94));
    assert_eq!(second.portfolio_return, Some(6.47));
    // 1.025 × 1.0647 − 1 = 9.13%；0050 同期 5%。
    assert_eq!(response.cumulative_return, Some(9.13));
    assert_eq!(response.etf_cumulative_return, Some(5.0));
    assert_eq!(response.excess_return, Some(4.13));
}

/// 查不到日線的持股以其餘持股的平均代替；空手報酬 0；換股當天（最新交易日）那期不列入累積；
/// 純策略不需要 0050。
#[test]
fn track_handles_missing_bars_cash_and_rebalance_today() {
    let missing = [record(
        day(9, 1),
        vec![holding("A", 50.0), holding("NO_BARS", 50.0)],
        0.0,
    )];
    let response = track("甲", &missing, Some(day(9, 29)), &bars(), &HashMap::new());
    assert_eq!(response.periods[0].holdings[1].return_percent, None);
    assert_eq!(response.periods[0].strategy_return, Some(10.0));

    let idle = [StrategyRebalanceRecord {
        etf_weight: 0,
        market_filtered: true,
        ..record(day(9, 1), Vec::new(), 100.0)
    }];
    let response = track("甲", &idle, Some(day(9, 29)), &bars(), &HashMap::new());
    assert_eq!(response.periods[0].portfolio_return, Some(0.0));
    assert_eq!(response.cumulative_return, Some(0.0));

    let today = [
        record(day(9, 1), vec![holding("A", 100.0)], 0.0),
        record(day(9, 29), vec![holding("C", 100.0)], 0.0),
    ];
    let response = track("甲", &today, Some(day(9, 29)), &bars(), &HashMap::new());
    assert_eq!(response.periods[1].strategy_return, None);
    // 只有第一期：0.5 × 10% + 0.5 × 2% = 6%。
    assert_eq!(response.cumulative_return, Some(6.0));

    let only_today = [record(day(10, 20), vec![holding("A", 100.0)], 0.0)];
    let response = track(
        "甲",
        &only_today,
        Some(day(10, 20)),
        &bars(),
        &HashMap::new(),
    );
    assert_eq!(response.cumulative_return, None);
    assert_eq!(response.etf_cumulative_return, None);

    let empty = track("甲", &[], None, &HashMap::new(), &HashMap::new());
    assert!(empty.from.is_none() && empty.to.is_none() && empty.periods.is_empty());
}
