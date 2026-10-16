//! 條件選股回測的績效彙總、個股貢獻與風險測試。

use super::*;

/// 個股貢獻：每期貢獻是報酬 ÷ 當期有報酬的持股數，各期相加；沒有報酬的期只算持有期數；
/// 正負各自排序並截斷，貢獻為 0 的不列。
#[test]
fn contributors_sum_equal_weight_shares() {
    use super::super::contributors::contributors;
    let period = |symbols: &[&str], returns: &[Option<f64>]| PeriodRow {
        holdings: symbols.len() as i64,
        symbols: symbols.iter().map(|symbol| (*symbol).to_owned()).collect(),
        returns: returns.to_vec(),
        ..row(2, 0, None, 0.0)
    };
    let rows = [
        // A 10、B -4、C 沒有報酬：有報酬的 2 檔，A 貢獻 5、B -2。
        period(&["A", "B", "C"], &[Some(10.0), Some(-4.0), None]),
        // A 6、D -9、E 0：3 檔各 1/3。
        period(&["A", "D", "E"], &[Some(6.0), Some(-9.0), Some(0.0)]),
        // 空手的期沒有任何持股。
        period(&[], &[]),
    ];
    let (held, best, worst) = contributors(&rows, 10);
    assert_eq!(held, 5);
    let brief = |items: &[super::super::contributors::Contribution]| {
        items
            .iter()
            .map(|item| {
                (
                    item.symbol.clone(),
                    item.periods_held,
                    item.contribution,
                    item.avg_return,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(brief(&best), [("A".to_owned(), 2, 7.0, Some(8.0))]);
    assert_eq!(
        brief(&worst),
        [
            ("D".to_owned(), 1, -3.0, Some(-9.0)),
            ("B".to_owned(), 1, -2.0, Some(-4.0)),
        ]
    );
    // 截斷：只留最差 1 檔。
    let (_, _, worst) = contributors(&rows, 1);
    assert_eq!(worst.len(), 1);
    assert_eq!(worst[0].symbol, "D");
    // C 只有持有沒有報酬、E 貢獻 0，都不在名單中但算持有過。
    assert!(
        best.iter()
            .chain(&worst)
            .all(|item| item.symbol != "C" && item.symbol != "E")
    );
}

/// 累積淨值連乘；回撤取淨值相對前高的最大跌幅。
#[test]
fn compound_tracks_equity_and_drawdown() {
    let (equity, drawdown) = compound([10.0, -20.0, 25.0]);
    // 1.1 → 0.88 → 1.1；前高 1.1，最低 0.88，回撤 -20%。
    assert!((equity - 1.1).abs() < 1e-12, "{equity}");
    assert!((drawdown + 0.2).abs() < 1e-12, "{drawdown}");
    assert_eq!(compound([]), (1.0, 0.0));
    let (_, rising) = compound([1.0, 2.0]);
    assert_eq!(rising, 0.0);
}

/// 空手的期數報酬算 0；勝率與勝過大盤以期數計；年化以每年 245 個交易日換算。
#[test]
fn summarize_counts_cash_periods_as_zero() {
    let rows = [
        row(2, 20, Some(10.0), 5.0),
        row(3, 0, None, -4.0),
        row(4, 10, Some(-5.0), -10.0),
        row(5, 18, Some(4.0), 6.0),
    ];
    let summary = summarize(&rows, &free(&rows), 20);
    assert_eq!(summary.periods, 4);
    assert_eq!(summary.invested_periods, 3);
    assert_eq!(summary.avg_holdings, Some(16.0));
    // 1.1 × 1 × 0.95 × 1.04 = 1.0868。
    assert_eq!(summary.total_return, Some(8.68));
    // 1.05 × 0.96 × 0.9 × 1.06 = 0.961632。
    assert_eq!(summary.benchmark_total_return, Some(-3.84));
    // 4 期 × 20 日 = 80 個交易日，約 0.3265 年。
    let years = 80.0 / 245.0;
    assert_eq!(
        summary.annualized_return,
        Some(round2((1.0868_f64.powf(1.0 / years) - 1.0) * 100.0))
    );
    // 期末淨值 1.1 → 1.1 → 1.045 → 1.0868：回撤 -5%。
    assert_eq!(summary.max_drawdown, Some(-5.0));
    // 1.05 → 1.008 → 0.9072：回撤 -13.6%。
    assert_eq!(summary.benchmark_max_drawdown, Some(-13.6));
    // 報酬 > 0：10、4；空手的 0 不算。
    assert_eq!(summary.win_rate, Some(0.5));
    // 勝過大盤：10 > 5、0 > -4、-5 > -10；4 < 6 不算。
    assert_eq!(summary.beat_market_rate, Some(0.75));
    assert_eq!(summary.avg_period_return, Some(2.25));
    assert_eq!(summary.avg_benchmark_return, Some(-0.75));
    // 沒有成本時扣成本後與原本相同。
    assert_eq!(summary.total_return_after_costs, Some(8.68));
    assert_eq!(summary.max_drawdown_after_costs, Some(-5.0));
    assert_eq!(summary.avg_turnover, Some(0.0));
    // 全期都沒有 0050 資料時，0050 相關欄位為 null。
    assert!(summary.etf_total_return.is_none() && summary.beat_etf_rate.is_none());
    assert!(summary.etf_volatility.is_none() && summary.etf_sharpe_ratio.is_none());
    // 每年 245 / 20 = 12.25 期。
    assert_eq!(
        summary.volatility,
        risk(&[10.0, 0.0, -5.0, 4.0], 12.25).map(|(volatility, _)| volatility)
    );
    assert!(summary.sharpe_ratio.is_some() && summary.benchmark_sharpe_ratio.is_some());
}

/// 0050 基準：缺值的期數算 0，勝過 0050 以期數計。
#[test]
fn summarize_compares_with_etf() {
    let rows = [
        dated(day(2024, 1, 2), Some(10.0), 5.0, Some(4.0)),
        dated(day(2024, 2, 1), Some(-2.0), -1.0, Some(-5.0)),
        dated(day(2024, 3, 1), Some(1.0), 0.0, None),
        dated(day(2024, 4, 1), None, 2.0, Some(2.0)),
    ];
    let summary = summarize(&rows, &free(&rows), 20);
    // 1.04 × 0.95 × 1 × 1.02 = 1.00776。
    assert_eq!(summary.etf_total_return, Some(0.78));
    // 1.04 → 0.988：回撤 -5%。
    assert_eq!(summary.etf_max_drawdown, Some(-5.0));
    // 10 > 4、-2 > -5、1 > 0（缺值算 0）；空手 0 < 2 不算。
    assert_eq!(summary.beat_etf_rate, Some(0.75));
    let years = 80.0 / 245.0;
    assert_eq!(
        summary.etf_annualized_return,
        Some(round2((1.00776_f64.powf(1.0 / years) - 1.0) * 100.0))
    );
}

/// 年度彙總：依換股日年度分組、各年從淨值 1 起算；整年缺 0050 資料為 null。
#[test]
fn yearly_groups_by_rebalance_year() {
    let rows = [
        dated(day(2022, 11, 1), Some(10.0), 5.0, None),
        dated(day(2022, 12, 1), Some(-10.0), -2.0, None),
        dated(day(2023, 1, 3), Some(5.0), 6.0, Some(3.0)),
        dated(day(2023, 6, 1), None, -1.0, Some(-1.0)),
        dated(day(2023, 12, 29), Some(2.0), 1.0, None),
    ];
    let costs: Vec<PeriodCost> = free(&rows)
        .into_iter()
        .map(|cost| PeriodCost {
            net_return: cost.net_return - 1.0,
            ..cost
        })
        .collect();
    let years = yearly(&rows, &costs);
    assert_eq!(years.len(), 2);
    let y2022 = &years[0];
    assert_eq!((y2022.year, y2022.periods), (2022, 2));
    // 1.1 × 0.9 = 0.99。
    assert_eq!(y2022.total_return, -1.0);
    // 1.09 × 0.89 = 0.9701。
    assert_eq!(y2022.total_return_after_costs, -2.99);
    // 1.05 × 0.98 = 1.029。
    assert_eq!(y2022.benchmark_total_return, 2.9);
    assert_eq!(y2022.etf_total_return, None);
    assert_eq!((y2022.win_rate, y2022.beat_market_rate), (0.5, 0.5));
    let y2023 = &years[1];
    assert_eq!((y2023.year, y2023.periods), (2023, 3));
    // 1.05 × 1 × 1.02 = 1.071。
    assert_eq!(y2023.total_return, 7.1);
    // 1.03 × 0.99 × 1 = 1.0197。
    assert_eq!(y2023.etf_total_return, Some(1.97));
    // 報酬 > 0：5、2；勝過基準：0 > -1、2 > 1。
    assert_eq!((y2023.win_rate, y2023.beat_market_rate), (0.6667, 0.6667));
    assert!(yearly(&[], &[]).is_empty());
}

/// 沒有任何一期時彙總全部為 `null`。
#[test]
fn summarize_without_periods_is_empty() {
    let summary = summarize(&[], &[], 20);
    assert_eq!((summary.periods, summary.invested_periods), (0, 0));
    assert!(summary.total_return.is_none() && summary.annualized_return.is_none());
    assert!(summary.max_drawdown.is_none() && summary.avg_holdings.is_none());
    assert!(summary.total_return_after_costs.is_none() && summary.avg_turnover.is_none());
    assert!(summary.etf_total_return.is_none() && summary.etf_max_drawdown.is_none());
}

/// 波動率與夏普值：樣本標準差 × √每年期數；不足 2 期為 None，沒有波動時夏普值為 None。
#[test]
fn risk_annualizes_volatility_and_sharpe() {
    // 平均 2、樣本標準差 √(((4−2)² + (0−2)²) / 1) = 2√2；每年 4 期 → 波動 4√2、夏普 2/(2√2)×2 = √2。
    let (volatility, sharpe) = risk(&[4.0, 0.0], 4.0).expect("兩期以上");
    assert_eq!(volatility, round2(4.0 * 2.0_f64.sqrt()));
    assert_eq!(sharpe, Some(round2(2.0_f64.sqrt())));
    assert_eq!(risk(&[3.0], 4.0), None);
    assert_eq!(risk(&[1.0, 1.0, 1.0], 4.0), Some((0.0, None)));
}
