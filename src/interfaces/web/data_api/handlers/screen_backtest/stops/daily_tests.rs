//! 拆出的既有回歸測試，保持原有案例與斷言。

use super::super::{
    portfolio::{PeriodCost, blend_etf},
    summary::summarize,
};
use super::*;
/// 建立相鄰日期上的還原收盤。
fn series(values: &[f64]) -> Arc<Vec<PriceBar>> {
    Arc::new(
        values
            .iter()
            .enumerate()
            .map(|(i, close)| PriceBar {
                date: NaiveDate::from_ymd_opt(2024, 1, 2 + i as u32).unwrap(),
                close: *close,
                high: *close,
                low: *close,
            })
            .collect(),
    )
}
/// 建立持有兩天的完整期數。
fn period(symbols: &[&str]) -> PeriodRow {
    PeriodRow {
        date: NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
        exit_date: Some(NaiveDate::from_ymd_opt(2024, 1, 4).unwrap()),
        symbols: symbols.iter().map(|s| s.to_string()).collect(),
        holdings: symbols.len() as i64,
        ..PeriodRow::default()
    }
}
/// 次日開盤成交：路徑從次一交易日開盤起算，出場日以開盤結算，終值等於開盤到開盤的報酬；
/// 缺比值時沿用收盤；0050 同樣處理；找不到次一交易日時標為不完整。
#[test]
fn next_open_daily_path_uses_open_endpoints() {
    let d = |n: u32| NaiveDate::from_ymd_opt(2024, 1, 2 + n).unwrap();
    // 01-02 換股；01-03 開 100 收 110；01-04 收 121；01-05 出場，開 133.1 收 110。
    let mut row = PeriodRow {
        date: d(0),
        exit_date: Some(d(3)),
        symbols: vec!["A".into()],
        holdings: 1,
        etf_return: Some(0.0),
        ..PeriodRow::default()
    };
    let bars = HashMap::from([
        ("A".into(), series(&[90.0, 110.0, 121.0, 110.0])),
        (
            super::super::ETF_BENCHMARK.into(),
            series(&[50.0, 50.0, 55.0, 50.0]),
        ),
    ]);
    let ratios: OpenRatios = HashMap::from([
        (("A".to_owned(), d(1)), 100.0 / 110.0),
        (("A".to_owned(), d(3)), 133.1 / 110.0),
        ((super::super::ETF_BENCHMARK.to_owned(), d(3)), 1.1),
    ]);
    let costs = [PeriodCost {
        net_return: 33.1,
        turnover: 0.0,
    }];
    populate_daily_returns(
        std::slice::from_mut(&mut row),
        &bars,
        StopRule::default(),
        1.0,
        &costs,
        Some(&ratios),
    );
    assert!(row.daily_complete);
    let expected = [10.0, 21.0, 33.1];
    assert_eq!(
        row.daily_returns.len(),
        expected.len(),
        "{:?}",
        row.daily_returns
    );
    for (got, want) in row.daily_returns.iter().zip(expected) {
        assert!((got - want).abs() < 1e-9, "{:?}", row.daily_returns);
    }
    // 0050 進場日沒有比值（沿用收盤 50），出場日開盤 55。
    assert!(
        (row.etf_daily_returns[2] - 10.0).abs() < 1e-9,
        "{:?}",
        row.etf_daily_returns
    );

    let calendar: std::collections::BTreeSet<_> = [d(0), d(1)].into_iter().collect();
    assert_eq!(entry_date(&calendar, d(0)), Some(d(1)));
    assert_eq!(entry_date(&calendar, d(1)), None);
    let mut last = PeriodRow {
        date: d(3),
        exit_date: Some(d(3)),
        symbols: vec!["A".into()],
        ..PeriodRow::default()
    };
    populate_daily_returns(
        std::slice::from_mut(&mut last),
        &bars,
        StopRule::default(),
        1.0,
        &costs,
        Some(&ratios),
    );
    assert!(!last.daily_complete, "沒有次一交易日的期數不算日頻路徑");
}

/// 成交金額加權與上限後現金不能退回等權。
#[test]
fn daily_paths_use_weights_and_cash() {
    let mut row = period(&["A", "B"]);
    row.weights = vec![0.9, 0.1];
    let bars = HashMap::from([
        ("A".into(), series(&[100.0, 50.0, 100.0])),
        ("B".into(), series(&[100.0, 100.0, 100.0])),
    ]);
    let costs = [PeriodCost {
        net_return: 0.0,
        turnover: 0.0,
    }];
    populate_daily_returns(
        std::slice::from_mut(&mut row),
        &bars,
        StopRule::default(),
        1.0,
        &costs,
        None,
    );
    assert_eq!(row.daily_returns, [0.0, -45.0, 0.0]);
    row.daily_returns.clear();
    row.daily_net_returns.clear();
    row.weights = vec![0.1, 0.1];
    row.cash = 0.8;
    populate_daily_returns(
        std::slice::from_mut(&mut row),
        &bars,
        StopRule::default(),
        1.0,
        &costs,
        None,
    );
    assert!((row.daily_returns[1] + 5.0).abs() < 1e-9);
}
/// 停損日扣賣出成本，之後價跌不再影響現金。
#[test]
fn stopped_paths_freeze_and_charge_on_exit() {
    let mut row = period(&["A"]);
    row.returns = vec![Some(-90.0)];
    let bars = HashMap::from([("A".into(), series(&[100.0, 80.0, 10.0]))]);
    let rule = StopRule {
        stop_loss: Some(10.0),
        trailing_stop: None,
    };
    apply_stops(std::slice::from_mut(&mut row), &bars, rule, 20);
    let costs = super::super::portfolio::trading_costs(std::slice::from_ref(&row), 1.0);
    populate_daily_returns(
        std::slice::from_mut(&mut row),
        &bars,
        rule,
        1.0,
        &costs,
        None,
    );
    assert!(
        row.daily_returns
            .iter()
            .zip([0.0, -20.0, -20.0])
            .all(|(a, b)| (a - b).abs() < 1e-9)
    );
    assert!((row.daily_net_returns[0] + 0.1425).abs() < 1e-9);
    assert!((row.daily_net_returns[2] - costs[0].net_return).abs() < 1e-9);
    let summary = summarize(&[row], &costs, 20);
    assert_eq!(summary.daily_max_drawdown, Some(-20.0));
    assert_eq!(summary.daily_max_drawdown_after_costs, Some(-20.5));
}
/// 只持有 0050 時不能帶入衛星策略的回撤。
#[test]
fn core_only_ignores_strategy_path_and_missing_strategy_prices() {
    let mut rows = [period(&["A"])];
    rows[0].avg_return = Some(-50.0);
    rows[0].etf_return = Some(0.0);
    let bars = HashMap::from([("0050".into(), series(&[100.0, 100.0, 100.0]))]);
    let mut costs = [PeriodCost {
        net_return: -50.0,
        turnover: 1.0,
    }];
    populate_daily_returns(&mut rows, &bars, StopRule::default(), 1.0, &costs, None);
    assert!(!rows[0].daily_complete);
    blend_etf(&mut rows, &mut costs, 1.0, 1.0);
    assert_eq!(summarize(&rows, &costs, 20).daily_max_drawdown, Some(0.0));
}
/// 缺少日線時明確保留 null，不以期末值冒充每日風險。
#[test]
fn missing_daily_prices_are_not_period_end_risk() {
    let mut rows = [period(&["A"])];
    let costs = [PeriodCost {
        net_return: 0.0,
        turnover: 0.0,
    }];
    populate_daily_returns(
        &mut rows,
        &HashMap::new(),
        StopRule::default(),
        1.0,
        &costs,
        None,
    );
    let summary = summarize(&rows, &costs, 20);
    assert!(summary.daily_max_drawdown.is_none());
    assert_eq!(summary.period_end_max_drawdown, Some(0.0));
}

fn wday(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).expect("有效日期")
}

fn wbar(m: u32, d: u32, close: f64) -> PriceBar {
    PriceBar {
        date: wday(m, d),
        close,
        high: close,
        low: close,
    }
}

/// 持有期間：換股日前 30 天到出場日；同一檔重疊或相鄰的期合併，未完成的期不讀；0050 涵蓋全部期間。
#[test]
fn holding_windows_merge_per_symbol() {
    let mk =
        |date: NaiveDate, exit: Option<NaiveDate>, symbols: &[&str], pending: bool| PeriodRow {
            date,
            exit_date: exit,
            pending,
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
            ..PeriodRow::default()
        };
    let rows = [
        mk(wday(3, 1), Some(wday(3, 29)), &["A", "B"], false),
        mk(wday(3, 29), Some(wday(4, 26)), &["A"], false),
        mk(wday(6, 3), Some(wday(6, 28)), &["B"], false),
        mk(wday(7, 1), None, &["C"], true),
    ];
    let windows = holding_windows(&rows);
    let lookback = chrono::Duration::days(WINDOW_LOOKBACK_DAYS);
    assert_eq!(
        windows,
        vec![
            (
                super::super::ETF_BENCHMARK.to_owned(),
                wday(3, 1) - lookback,
                wday(6, 28)
            ),
            ("A".to_owned(), wday(3, 1) - lookback, wday(4, 26)),
            ("B".to_owned(), wday(3, 1) - lookback, wday(3, 29)),
            ("B".to_owned(), wday(6, 3) - lookback, wday(6, 28)),
        ]
    );
    assert!(holding_windows(&rows[3..]).is_empty());
}

/// 分段還原：段內的相對報酬與整段還原相同；段外的日線不會出現。
#[test]
fn adjust_windows_keeps_in_window_returns() {
    let series = [
        wbar(1, 2, 100.0),
        wbar(1, 3, 90.0),
        wbar(2, 1, 50.0),
        wbar(2, 2, 45.0),
        wbar(2, 5, 50.0),
    ];
    let events = [PriceAdjustment {
        date: wday(2, 2),
        cash_dividend: 5.0,
        stock_dividend: 0.0,
        share_ratio: 1.0,
    }];
    let windows = vec![
        ("A".to_owned(), wday(1, 1), wday(1, 3)),
        ("A".to_owned(), wday(2, 1), wday(2, 5)),
        ("Z".to_owned(), wday(1, 1), wday(12, 31)),
    ];
    let adjusted = adjust_windows(&series, &windows, "A", &events);
    assert_eq!(adjusted.len(), 5);
    let full = adjust_bars(&series, &events);
    let ratio = |bars: &[PriceBar], a: usize, b: usize| bars[b].close / bars[a].close;
    for (a, b) in [(0, 1), (2, 3), (2, 4), (3, 4)] {
        assert!(
            (ratio(&adjusted, a, b) - ratio(&full, a, b)).abs() < 1e-12,
            "{a}->{b}"
        );
    }
    assert!(adjust_windows(&series, &windows[..1], "A", &events).len() == 2);
}
