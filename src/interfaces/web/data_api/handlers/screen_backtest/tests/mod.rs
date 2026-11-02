//! 條件選股回測的單元測試：共用的測試資料與期間解析、SQL、目前持股；其餘依主題分在子模組。

use super::current::split_current;
use super::portfolio::{Weighting, blend_etf, select_holdings, trading_costs};
use super::sql::{BACKTEST_SQL, BACKTEST_WITH_CHIP_SQL};
use super::summary::{compound, risk};
use super::validate::{parse_excluded_industries, resolve_backtest_period, validate_fundamentals};
use super::*;

mod costs;
mod holdings;
mod major;
mod offsets;
mod sql;
mod summary;
mod validation;
mod weights;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

fn row(d: u32, holdings: i64, avg: Option<f64>, benchmark: f64) -> PeriodRow {
    PeriodRow {
        entry_date: None,
        date: day(2024, 1, d),
        exit_date: None,
        pending: false,
        market_filtered: false,
        holdings,
        avg_return: avg,
        benchmark_return: Some(benchmark),
        etf_return: None,
        symbols: Vec::new(),
        returns: Vec::new(),
        trade_values: Vec::new(),
        weights: Vec::new(),
        cash: 0.0,
        stopped: Vec::new(),
        daily_returns: Vec::new(),
        daily_net_returns: Vec::new(),
        etf_daily_returns: Vec::new(),
        daily_complete: false,
        etf_daily_complete: false,
        industries: Vec::new(),
    }
}

/// 指定年月日與 0050 報酬的一期。
fn dated(date: NaiveDate, avg: Option<f64>, benchmark: f64, etf: Option<f64>) -> PeriodRow {
    PeriodRow {
        entry_date: None,
        date,
        holdings: i64::from(avg.is_some()),
        etf_return: etf,
        ..row(2, 0, avg, benchmark)
    }
}

fn holding(symbols: &[&str], avg: f64) -> PeriodRow {
    PeriodRow {
        entry_date: None,
        holdings: symbols.len() as i64,
        symbols: symbols.iter().map(|symbol| (*symbol).to_owned()).collect(),
        ..row(2, 0, Some(avg), 0.0)
    }
}

/// 沒有成本的報酬（換手率另計），用來測試彙總。
fn free(rows: &[PeriodRow]) -> Vec<PeriodCost> {
    rows.iter()
        .map(|row| PeriodCost {
            turnover: 0.0,
            net_return: row.avg_return.unwrap_or(0.0),
        })
        .collect()
}

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
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

/// 一期候選：代號依相對強度由高到低，報酬一一對應。
fn candidates(d: u32, list: &[(&str, Option<f64>)]) -> CandidateRow {
    CandidateRow {
        entry_date: None,
        date: day(2024, 1, d),
        liquidity_start: day(2024, 1, d),
        exit_date: None,
        pending: false,
        elapsed_days: 0,
        market_ok: true,
        benchmark_return: Some(1.0),
        etf_return: Some(0.5),
        candidates: list
            .iter()
            .map(|(symbol, _)| (*symbol).to_owned())
            .collect(),
        candidate_returns: list.iter().map(|(_, ret)| *ret).collect(),
        candidate_industries: Vec::new(),
        candidate_trade_values: Vec::new(),
        hold_symbols: Vec::new(),
        hold_returns: Vec::new(),
        hold_industries: Vec::new(),
        hold_trade_values: Vec::new(),
    }
}

/// 加上只符合續抱條件的股票（相對強度不低於門檻）。
fn with_hold(mut row: CandidateRow, hold: &[(&str, Option<f64>)]) -> CandidateRow {
    row.hold_symbols = hold
        .iter()
        .map(|(symbol, _)| (*symbol).to_owned())
        .collect();
    row.hold_returns = hold.iter().map(|(_, ret)| *ret).collect();
    row
}

/// 設定候選股與續抱名單的產業編號（依序對應）。
fn with_industries(mut row: CandidateRow, candidates: &[i32], hold: &[i32]) -> CandidateRow {
    row.candidate_industries = candidates.to_vec();
    row.hold_industries = hold.to_vec();
    row
}

/// 最後一期未滿持有天數時移出各期、保留持股與換手；未標記或沒有期數時不動。
#[test]
fn split_current_takes_pending_last_period() {
    let mut pending = candidates(16, &[("A", None), ("C", None)]);
    pending.pending = true;
    let mut rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(4.0)), ("B", Some(2.0))]),
            pending,
        ],
        2,
        2,
        None,
    );
    assert!(rows[1].pending && !rows[0].pending);
    let mut costs = trading_costs(&rows, 1.0);
    let (row, cost) = split_current(&mut rows, &mut costs).expect("有目前持股");
    assert_eq!(row.symbols, ["A", "C"]);
    assert_eq!(row.avg_return, None);
    assert!(near(cost.turnover, 0.5));
    assert_eq!((rows.len(), costs.len()), (1, 1));

    // 最後一期已滿：不移出。
    assert!(split_current(&mut rows, &mut costs).is_none());
    assert_eq!(rows.len(), 1);
    assert!(split_current(&mut Vec::new(), &mut Vec::new()).is_none());
}

/// 換股日至今的報酬：以換股日（或之前最近一根）還原收盤為基準；除權息後的價格向後還原。
#[test]
fn return_since_uses_adjusted_close_from_rebalance_day() {
    use super::current::return_since;
    use crate::domain::quote::technical::{PriceAdjustment, PriceBar};
    let bar = |d: u32, close: f64| PriceBar {
        date: day(2024, 1, d),
        close,
        high: close,
        low: close,
    };
    let bars = [bar(2, 100.0), bar(3, 110.0), bar(5, 99.0)];
    // 換股日 01-03：110 → 99，跌 10%。
    let ret = return_since(&bars, &[], day(2024, 1, 3)).expect("有日線");
    assert!(near(ret, -10.0), "{ret}");
    // 換股日 01-04 沒有成交：往前取 01-03。
    assert_eq!(return_since(&bars, &[], day(2024, 1, 4)), Some(ret));
    // 01-05 除息 11 元：01-03 的 110 還原成 99，報酬 0。
    let dividend = PriceAdjustment {
        date: day(2024, 1, 5),
        cash_dividend: 11.0,
        stock_dividend: 0.0,
        share_ratio: 1.0,
    };
    let adjusted = return_since(&bars, &[dividend], day(2024, 1, 3)).expect("有日線");
    assert!(near(adjusted, 0.0), "{adjusted}");
    // 換股日早於全部日線、沒有日線、價格為 0 時都沒有報酬。
    assert_eq!(return_since(&bars, &[], day(2024, 1, 1)), None);
    assert_eq!(return_since(&[], &[], day(2024, 1, 3)), None);
    assert_eq!(
        return_since(&[bar(2, 0.0), bar(3, 5.0)], &[], day(2024, 1, 2)),
        None
    );
}

/// 持股依權重的報酬：只算有報酬的持股，乘上持股比例；都沒有報酬為 `None`。
#[test]
fn weighted_return_uses_shares_and_cash() {
    use super::current::weighted_return;
    let mut row = holding(&["A", "B", "C"], 0.0);
    row.weights = vec![2.0, 1.0, 1.0];
    let returns: std::collections::HashMap<String, f64> =
        [("A".to_owned(), 10.0), ("B".to_owned(), -2.0)].into();
    // (10×0.5 − 2×0.25) ÷ 0.75 = 6；C 沒有報酬不列入。
    let ret = weighted_return(&row, &returns).expect("有報酬");
    assert!(near(ret, 6.0), "{ret}");
    row.cash = 0.5;
    let ret = weighted_return(&row, &returns).expect("有報酬");
    assert!(near(ret, 3.0), "{ret}");
    assert_eq!(
        weighted_return(&row, &std::collections::HashMap::new()),
        None
    );
}

mod query_equivalence;
