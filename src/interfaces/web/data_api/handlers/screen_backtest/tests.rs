//! 條件選股回測的期間解析、累積報酬、回撤與彙總測試。

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

fn row(d: u32, holdings: i64, avg: Option<f64>, benchmark: f64) -> PeriodRow {
    PeriodRow {
        date: day(2024, 1, d),
        holdings,
        avg_return: avg,
        benchmark_return: Some(benchmark),
        symbols: Vec::new(),
    }
}

/// 起日預設迄日往前 1,095 天；起日晚於迄日或期間超過 3,653 天回錯誤。
#[test]
fn resolve_backtest_period_defaults_and_limits() {
    let to = day(2026, 10, 14);
    assert_eq!(resolve_backtest_period(None, to), Ok(day(2023, 10, 15)));
    assert_eq!(
        resolve_backtest_period(Some(day(2016, 10, 14)), to),
        Ok(day(2016, 10, 14))
    );
    assert_eq!(
        resolve_backtest_period(Some(day(2016, 10, 13)), to),
        Err("回測期間最長 3,653 天")
    );
    assert_eq!(
        resolve_backtest_period(Some(day(2026, 10, 15)), to),
        Err("from 不可晚於 to")
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
    let summary = summarize(&rows, 20);
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
}

/// 沒有任何一期時彙總全部為 `null`。
#[test]
fn summarize_without_periods_is_empty() {
    let summary = summarize(&[], 20);
    assert_eq!((summary.periods, summary.invested_periods), (0, 0));
    assert!(summary.total_return.is_none() && summary.annualized_return.is_none());
    assert!(summary.max_drawdown.is_none() && summary.avg_holdings.is_none());
}

/// 每期輸出：報酬四捨五入到小數兩位，空手為 `null`。
#[test]
fn period_rounds_returns() {
    let mut first = row(2, 3, Some(1.234_5), -0.555);
    first.symbols = vec!["2330".to_owned()];
    let output = period(first);
    assert_eq!(output.date, "2024-01-02");
    assert_eq!(output.avg_return, Some(1.23));
    assert_eq!(output.benchmark_return, Some(-0.56));
    assert_eq!(output.symbols, ["2330"]);
    assert_eq!(period(row(3, 0, None, 1.0)).avg_return, None);
}

/// 兩種 SQL 都以 `WITH` 開頭，籌碼版含連續買超的 CTE 與 `$11`／`$12`。
#[test]
fn backtest_sql_variants_are_well_formed() {
    assert!(BACKTEST_SQL.trim_start().starts_with("WITH\ndays AS ("));
    assert!(!BACKTEST_SQL.contains("$11"));
    assert!(
        BACKTEST_WITH_CHIP_SQL
            .trim_start()
            .starts_with("WITH\nchip_runs AS (")
    );
    assert!(BACKTEST_WITH_CHIP_SQL.contains("),\ndays AS ("));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("c.foreign_buy >= $11"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("c.trust_buy >= $12"));
}
