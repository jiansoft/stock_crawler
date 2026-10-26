//! 次日開盤停損的成交端點、停牌順延與費稅路徑驗證。

use super::super::super::portfolio::trading_costs;
use super::*;

/// 固定測試交易日。
fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, d).unwrap()
}

/// 由還原收盤與開盤建立測試日線；開盤缺值代表當天不可賣出。
fn prices(values: &[(u32, f64, Option<f64>)]) -> (Vec<PriceBar>, OpenRatios) {
    let bars = values
        .iter()
        .map(|&(d, close, _)| PriceBar {
            date: day(d),
            close,
            high: close,
            low: close,
        })
        .collect();
    let ratios = values
        .iter()
        .filter_map(|&(d, close, open)| open.map(|open| (("A".to_owned(), day(d)), open / close)))
        .collect();
    (bars, ratios)
}

/// 收盤觸發後使用下一天開盤；當天收盤與觸發收盤都不能作為賣價。
#[test]
fn trigger_then_next_open_and_daily_cash_match_costs() {
    let (series, ratios) = prices(&[
        (1, 100.0, Some(100.0)),
        (2, 88.0, Some(99.0)),
        (3, 90.0, Some(85.0)),
        (4, 60.0, Some(75.0)),
        (5, 80.0, Some(80.0)),
    ]);
    let rule = StopRule {
        stop_loss: Some(10.0),
        trailing_stop: None,
    };
    let sale = stop_exit(&series, "A", day(1), day(5), rule, &ratios).unwrap();
    assert_eq!(sale.date, day(3));
    assert!((sale.ret + 15.0).abs() < 1e-9);
    let bars = HashMap::from([("A".to_owned(), Arc::new(series))]);
    let mut rows = vec![PeriodRow {
        date: day(1),
        entry_date: Some(day(1)),
        exit_date: Some(day(5)),
        holdings: 1,
        symbols: vec!["A".into()],
        returns: vec![Some(-20.0)],
        avg_return: Some(-20.0),
        ..Default::default()
    }];
    apply_next_open_stops(&mut rows, &bars, rule, &ratios);
    assert_eq!(rows[0].stopped, [true]);
    let costs = trading_costs(&rows, 1.0);
    super::super::populate_daily_returns(&mut rows, &bars, rule, 1.0, &costs, Some(&ratios));
    assert!(rows[0].daily_complete);
    let path = &rows[0].daily_net_returns;
    assert!((path.last().unwrap() - costs[0].net_return).abs() < 1e-9);
    assert!((costs[0].net_return - (-15.0 - 0.1425 - 0.85 * 0.4425)).abs() < 1e-9);
    assert_eq!(path[2], path[3]);
    assert_eq!(path[3], path[4]);
    assert!(path[1] > path[2], "觸發日仍按收盤估值，費稅在賣出日才扣");
}

/// 次日缺開盤或沒有日線都順延；順延到期末或之後不建立提前成交。
#[test]
fn suspended_sale_rolls_forward_without_crossing_period_end() {
    let (series, ratios) = prices(&[
        (1, 100.0, Some(100.0)),
        (2, 88.0, Some(99.0)),
        (3, 89.0, None),
        (5, 72.0, Some(70.0)),
        (6, 50.0, Some(60.0)),
    ]);
    let rule = StopRule {
        stop_loss: Some(10.0),
        trailing_stop: None,
    };
    let sale = stop_exit(&series, "A", day(1), day(6), rule, &ratios).unwrap();
    assert_eq!(sale.date, day(5));
    assert!((sale.ret + 30.0).abs() < 1e-9);
    assert!(stop_exit(&series, "A", day(1), day(3), rule, &ratios).is_none());
    assert!(stop_exit(&series, "A", day(1), day(5), rule, &ratios).is_none());
}

/// 開盤買進當天的收盤即可觸發；移動停利峰值只用收盤更新。
#[test]
fn entry_close_and_trailing_peak_can_trigger() {
    let (series, ratios) = prices(&[
        (1, 88.0, Some(100.0)),
        (2, 90.0, Some(95.0)),
        (3, 100.0, Some(100.0)),
    ]);
    let rule = StopRule {
        stop_loss: Some(10.0),
        trailing_stop: None,
    };
    let sale = stop_exit(&series, "A", day(1), day(3), rule, &ratios).unwrap();
    assert_eq!(sale.date, day(2));
    assert!((sale.ret + 5.0).abs() < 1e-9);
    let (series, ratios) = prices(&[
        (1, 100.0, Some(100.0)),
        (2, 120.0, Some(100.0)),
        (3, 107.0, Some(130.0)),
        (4, 110.0, Some(112.0)),
        (5, 100.0, Some(100.0)),
    ]);
    let rule = StopRule {
        stop_loss: None,
        trailing_stop: Some(10.0),
    };
    let sale = stop_exit(&series, "A", day(1), day(5), rule, &ratios).unwrap();
    assert_eq!(sale.date, day(4));
    assert!((sale.ret - 12.0).abs() < 1e-9);
    assert!(stop_exit(&series, "A", day(1), day(5), StopRule::default(), &ratios).is_none());
    assert!(stop_exit(&[], "A", day(1), day(5), rule, &ratios).is_none());
    assert!(stop_exit(&series, "A", day(5), day(1), rule, &ratios).is_none());
}
