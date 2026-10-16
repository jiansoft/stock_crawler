//! 條件選股回測的期間解析、累積報酬、回撤與彙總測試。

use super::summary::{compound, risk};
use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

fn row(d: u32, holdings: i64, avg: Option<f64>, benchmark: f64) -> PeriodRow {
    PeriodRow {
        date: day(2024, 1, d),
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
    }
}

/// 指定年月日與 0050 報酬的一期。
fn dated(date: NaiveDate, avg: Option<f64>, benchmark: f64, etf: Option<f64>) -> PeriodRow {
    PeriodRow {
        date,
        holdings: i64::from(avg.is_some()),
        etf_return: etf,
        ..row(2, 0, avg, benchmark)
    }
}

fn holding(symbols: &[&str], avg: f64) -> PeriodRow {
    PeriodRow {
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

/// 個股貢獻：每期貢獻是報酬 ÷ 當期有報酬的持股數，各期相加；沒有報酬的期只算持有期數；
/// 正負各自排序並截斷，貢獻為 0 的不列。
#[test]
fn contributors_sum_equal_weight_shares() {
    use super::contributors::contributors;
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
    let brief = |items: &[super::contributors::Contribution]| {
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

/// 0050 核心部位：報酬與扣成本後報酬依比例混合、換手只算策略部位；空手與 0050 缺資料的期數照算；
/// 比例 0 不變、100 等於只持有 0050。
#[test]
fn blend_etf_mixes_core_and_strategy() {
    let mut rows = [
        dated(day(2024, 1, 2), Some(10.0), 0.0, Some(4.0)),
        dated(day(2024, 2, 1), None, 0.0, Some(-2.0)),
        dated(day(2024, 3, 1), Some(6.0), 0.0, None),
    ];
    let mut costs = [
        PeriodCost {
            turnover: 1.0,
            net_return: 9.8,
        },
        PeriodCost {
            turnover: 0.0,
            net_return: -0.4,
        },
        PeriodCost {
            turnover: 0.5,
            net_return: 5.9,
        },
    ];
    let (original_rows, original_costs) = (rows.clone(), costs);
    blend_etf(&mut rows, &mut costs, 0.0, 1.0);
    assert_eq!(
        (rows.clone(), costs),
        (original_rows.clone(), original_costs)
    );

    blend_etf(&mut rows, &mut costs, 0.25, 1.0);
    let avg: Vec<Option<f64>> = rows.iter().map(|row| row.avg_return).collect();
    assert_eq!(avg, [Some(8.5), Some(-0.5), Some(4.5)]);
    // 第一期 0050 第一次買進不計成本。
    assert!(
        near(costs[0].net_return, 0.25 * 4.0 + 0.75 * 9.8),
        "{costs:?}"
    );
    // 第一期策略漲 10%、0050 漲 4%：0050 比例掉到 0.26 ÷ 1.085，換股時減碼策略（手續費與證交稅）、
    // 買進 0050（手續費）。
    let drift = 0.25 - 0.26 / 1.085;
    assert!(
        near(
            costs[1].net_return,
            0.25 * -2.0 + 0.75 * -0.4 - drift * (0.1425 + 0.4425)
        ),
        "{costs:?}"
    );
    // 第二期策略空手：0050 跌 2% 後比例低於 0.25，減碼的是現金，只付買進 0050 的手續費。
    let cash_drift = 0.25 - 0.245 / 0.995;
    assert!(
        near(costs[2].net_return, 0.75 * 5.9 - cash_drift * 0.1425),
        "{costs:?}"
    );
    let turnover: Vec<f64> = costs.iter().map(|cost| cost.turnover).collect();
    assert!(near(turnover[0], 0.75), "{turnover:?}");
    assert!(near(turnover[1], drift), "{turnover:?}");
    assert!(near(turnover[2], 0.375 + cash_drift), "{turnover:?}");
    // 持股與報酬明細只看策略部位，不受影響。
    assert_eq!(rows[0].holdings, original_rows[0].holdings);

    // 比例 100% 不用調整，等於只持有 0050。
    let (mut rows, mut costs) = (original_rows.clone(), original_costs);
    blend_etf(&mut rows, &mut costs, 1.0, 1.0);
    let net: Vec<f64> = costs.iter().map(|cost| cost.net_return).collect();
    assert_eq!(net, [4.0, -2.0, 0.0]);

    // 0050 漲得比策略多：賣出 0050（手續費與 ETF 證交稅）、加碼策略（手續費）。
    let mut rows = [
        dated(day(2024, 1, 2), Some(0.0), 0.0, Some(10.0)),
        dated(day(2024, 2, 1), Some(1.0), 0.0, Some(0.0)),
    ];
    let mut costs = free(&rows);
    blend_etf(&mut rows, &mut costs, 0.5, 0.0);
    let excess = 0.55 / 1.05 - 0.5;
    assert!(
        near(
            costs[1].net_return,
            0.5 - excess * super::portfolio::ETF_TAX_RATE
        ),
        "{costs:?}"
    );
    assert!(near(costs[1].turnover, excess), "{costs:?}");
}

/// 換股成本：第一期全部買進；賣出付手續費與證交稅、買進付手續費；續抱且比例不變的不計，
/// 漲跌後調回目標比例的部分要計；空手時賣光。
#[test]
fn trading_costs_follow_turnover() {
    let rows = [
        holding(&["A", "B"], 10.0),
        holding(&["B", "C"], 5.0),
        holding(&["B", "C"], 1.0),
        row(3, 0, None, 0.0),
    ];
    let costs = trading_costs(&rows, 1.0);
    let turnover: Vec<f64> = costs.iter().map(|cost| cost.turnover).collect();
    assert_eq!(turnover, [1.0, 0.5, 0.0, 0.0]);
    // 第一期：買進全部，付 0.1425%。
    assert!(near(costs[0].net_return, 10.0 - 0.1425), "{costs:?}");
    // 第二期：賣出 A（一半）付 0.1425% + 0.3%，買進 C（一半）付 0.1425%。
    assert!(
        near(costs[1].net_return, 5.0 - 0.5 * 0.4425 - 0.5 * 0.1425),
        "{costs:?}"
    );
    assert!(near(costs[2].net_return, 1.0), "{costs:?}");
    // 空手：賣光上一期持股。
    assert!(near(costs[3].net_return, -0.4425), "{costs:?}");
    // 手續費 2.8 折只影響手續費。
    let discounted = trading_costs(&rows[..2], 0.28);
    assert!(
        near(discounted[0].net_return, 10.0 - 0.0399),
        "{discounted:?}"
    );
    assert!(
        near(
            discounted[1].net_return,
            5.0 - 0.5 * (0.0399 + 0.3) - 0.5 * 0.0399
        ),
        "{discounted:?}"
    );

    // 續抱股票漲跌後調回等權：A 漲 20%、B 持平，期末 A 占 0.6 ÷ 1.1，換股時減碼 A、加碼 B。
    let mut drifting = [holding(&["A", "B"], 10.0), holding(&["A", "B"], 2.0)];
    drifting[0].returns = vec![Some(20.0), Some(0.0)];
    let costs_drift = trading_costs(&drifting, 1.0);
    let trimmed = 0.6 / 1.1 - 0.5;
    assert!(near(costs_drift[1].turnover, trimmed), "{costs_drift:?}");
    assert!(
        near(
            costs_drift[1].net_return,
            2.0 - trimmed * 0.4425 - trimmed * 0.1425
        ),
        "{costs_drift:?}"
    );

    let summary = summarize(&rows, &costs, 20);
    // 換手率只看有持股的三期：(1 + 0.5 + 0) / 3。
    assert_eq!(summary.avg_turnover, Some(0.5));
    let expected = (1.0 + costs[0].net_return / 100.0)
        * (1.0 + costs[1].net_return / 100.0)
        * (1.0 + costs[2].net_return / 100.0)
        * (1.0 + costs[3].net_return / 100.0);
    assert_eq!(
        summary.total_return_after_costs,
        Some(round2((expected - 1.0) * 100.0))
    );
    assert!(summary.total_return_after_costs < summary.total_return);
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

/// 每期輸出：報酬四捨五入到小數兩位，空手為 `null`；代號只列前 10 檔。
#[test]
fn period_rounds_returns() {
    let mut first = row(2, 12, Some(1.234_5), -0.555);
    first.symbols = (0..12).map(|i| format!("{}", 2300 + i)).collect();
    let cost = PeriodCost {
        turnover: 1.0 / 3.0,
        net_return: 1.091_23,
    };
    let output = period((first, cost));
    assert_eq!(output.date, "2024-01-02");
    assert_eq!(output.avg_return, Some(1.23));
    assert_eq!(output.return_after_costs, 1.09);
    assert_eq!(output.turnover, 0.3333);
    assert_eq!(output.benchmark_return, Some(-0.56));
    assert_eq!(output.etf_return, None);
    assert_eq!(output.symbols.len(), 10);
    assert_eq!(output.symbols[0], "2300");
    let cash = PeriodCost {
        turnover: 0.0,
        net_return: 0.0,
    };
    assert_eq!(period((row(3, 0, None, 1.0), cash)).avg_return, None);
}

/// 兩種 SQL 都以 `WITH` 開頭、含大盤濾網 `$11`、續抱門檻 `$12`、市場廣度門檻 `$13`、換股日偏移 `$14`
/// 、流動性門檻 `$15`、排序方式 `$16` 與加權方式 `$17`，籌碼版另含連續買超的 CTE 與 `$18`／`$19`；沒有籌碼的版本
/// 排序用的連續買超欄位恆為 `NULL`。
#[test]
fn backtest_sql_variants_are_well_formed() {
    assert!(BACKTEST_SQL.trim_start().starts_with("WITH\ndays AS ("));
    assert!(BACKTEST_SQL.contains("COALESCE($11::int, 1)"));
    assert!(BACKTEST_SQL.contains("rs_score >= $12"));
    assert!(BACKTEST_SQL.contains("$13::int IS NULL OR br.above"));
    assert!(BACKTEST_SQL.contains("WHERE i >= $14::int AND (i - $14::int) % $4::int = 0"));
    assert!(BACKTEST_SQL.contains("$15::int8 IS NULL OR"));
    assert!(BACKTEST_SQL.contains("BETWEEN f.liquidity_start AND f.date"));
    assert!(BACKTEST_SQL.contains("$15::int8 IS NOT NULL OR $16::text = 'trade_value'"));
    assert!(BACKTEST_SQL.contains("ORDER BY CASE $16::text"));
    assert!(BACKTEST_SQL.contains("NULL::float8 AS foreign_buy, NULL::float8 AS trust_buy"));
    assert!(BACKTEST_SQL.contains("OR $17::text <> 'equal' THEN ("));
    assert!(
        BACKTEST_SQL
            .contains("CASE WHEN $17::text <> 'equal' THEN (\n                   SELECT AVG")
    );
    assert!(BACKTEST_SQL.contains("AS candidate_trade_values"));
    assert!(BACKTEST_SQL.contains("AS hold_trade_values"));
    assert!(!BACKTEST_SQL.contains("$18"));
    assert!(BACKTEST_SQL.contains("b.samples = 0 AS pending"));
    assert!(BACKTEST_SQL.contains("OR b.date = (SELECT MAX(date) FROM rebal)"));
    assert!(
        BACKTEST_WITH_CHIP_SQL
            .trim_start()
            .starts_with("WITH\nchip AS (")
    );
    assert!(BACKTEST_WITH_CHIP_SQL.contains("),\ndays AS ("));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("c.foreign_buy >= $18"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("c.trust_buy >= $19"));
    assert!(!BACKTEST_WITH_CHIP_SQL.contains("$20"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("LEFT JOIN chip c ON"));
    assert!(BACKTEST_WITH_CHIP_SQL.contains("COALESCE(c.foreign_buy, 0)::float8 AS foreign_buy"));
    for rank_by in RANK_BY {
        assert!(rank_by == "rs_score" || BACKTEST_SQL.contains(&format!("WHEN '{rank_by}' THEN")));
    }
}

/// 一期候選：代號依相對強度由高到低，報酬一一對應。
fn candidates(d: u32, list: &[(&str, Option<f64>)]) -> CandidateRow {
    CandidateRow {
        date: day(2024, 1, d),
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

/// 不緩衝時每期取前 limit 檔；沒有報酬的股票占名額但不列入平均；沒有候選為空手。
#[test]
fn select_holdings_without_buffer_takes_top_limit() {
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(10.0)), ("B", None), ("C", Some(2.0))]),
            candidates(3, &[]),
            candidates(4, &[("D", None)]),
        ],
        2,
        2,
        None,
    );
    assert_eq!(rows[0].symbols, ["A", "B"]);
    assert_eq!((rows[0].holdings, rows[0].avg_return), (2, Some(10.0)));
    assert_eq!(
        (rows[0].benchmark_return, rows[0].etf_return),
        (Some(1.0), Some(0.5))
    );
    assert_eq!((rows[1].holdings, rows[1].avg_return), (0, None));
    assert!(rows[1].symbols.is_empty());
    assert_eq!((rows[2].holdings, rows[2].avg_return), (1, None));
}

/// 緩衝：上一期持股只要仍在前 keep_rank 名就續抱，空出的名額依排名補滿；輸出依排名排序。
#[test]
fn select_holdings_keeps_previous_within_buffer() {
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(1.0)), ("B", Some(2.0)), ("C", Some(3.0))]),
            // A 掉到第 4 名仍在緩衝內續抱；B 跌出名單賣出，補第 1 名 D。
            candidates(
                3,
                &[
                    ("D", Some(4.0)),
                    ("C", Some(5.0)),
                    ("E", Some(6.0)),
                    ("A", Some(7.0)),
                ],
            ),
            // A 掉到第 5 名超出緩衝：換成排名最高、尚未持有的 E。
            candidates(
                4,
                &[
                    ("C", Some(1.0)),
                    ("D", Some(1.0)),
                    ("E", Some(1.0)),
                    ("F", Some(1.0)),
                    ("A", Some(1.0)),
                ],
            ),
        ],
        2,
        4,
        None,
    );
    assert_eq!(rows[0].symbols, ["A", "B"]);
    assert_eq!(rows[1].symbols, ["D", "A"]);
    assert_eq!(rows[1].avg_return, Some(5.5));
    assert_eq!(rows[2].symbols, ["C", "D"]);
    // 續抱的股票比名額多時只留排名最前的 limit 檔。
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", None), ("B", None), ("C", None)]),
            candidates(3, &[("X", None), ("C", None), ("B", None), ("A", None)]),
        ],
        3,
        4,
        None,
    );
    assert_eq!(rows[1].symbols, ["C", "B", "A"]);
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

/// 大盤濾網沒過的那期空手並賣光；下一期重新依排名買進（不算續抱）。
#[test]
fn select_holdings_goes_to_cash_when_market_filtered() {
    let mut filtered = candidates(3, &[("A", Some(5.0)), ("B", Some(1.0))]);
    filtered.market_ok = false;
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(1.0)), ("B", Some(2.0))]),
            filtered,
            candidates(4, &[("C", Some(3.0)), ("A", Some(4.0))]),
        ],
        1,
        2,
        None,
    );
    assert_eq!(
        (rows[0].symbols.clone(), rows[0].market_filtered),
        (vec!["A".to_owned()], false)
    );
    assert!(rows[1].market_filtered && rows[1].symbols.is_empty());
    assert_eq!((rows[1].holdings, rows[1].avg_return), (0, None));
    // 上一期空手，A 雖在緩衝內也不是續抱，改買第 1 名 C。
    assert_eq!(rows[2].symbols, ["C"]);
    let summary = summarize(&rows, &free(&rows), 20);
    assert_eq!(summary.market_filtered_periods, 1);
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

/// 續抱條件：上一期持股不再符合選股條件、但相對強度仍達門檻就續抱，報酬取續抱名單的報酬；
/// 跌破門檻才換股。續抱股排在候選股之後，名額不足時先留候選股。
#[test]
fn select_holdings_keeps_stocks_meeting_hold_rule() {
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(1.0)), ("B", Some(2.0))]),
            // A、B 都不再是候選（例如連買中斷）；A 的 RS 仍達門檻續抱，B 沒有，補第 1 名 C。
            with_hold(
                candidates(3, &[("C", Some(3.0)), ("D", Some(4.0))]),
                &[("C", Some(3.0)), ("A", Some(5.0))],
            ),
            // A 跌破門檻：換成 D。
            with_hold(
                candidates(4, &[("C", Some(1.0)), ("D", Some(2.0))]),
                &[("C", Some(1.0))],
            ),
        ],
        2,
        2,
        None,
    );
    assert_eq!(rows[1].symbols, ["C", "A"]);
    assert_eq!(rows[1].avg_return, Some(4.0));
    assert_eq!(rows[2].symbols, ["C", "D"]);

    // 續抱的股票多於名額時先留候選股。
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", None), ("B", None)]),
            with_hold(
                candidates(3, &[("B", None), ("X", None)]),
                &[("A", None), ("B", None)],
            ),
        ],
        1,
        1,
        None,
    );
    assert_eq!(rows[0].symbols, ["A"]);
    // A 只符合續抱條件、B 是候選第 1 名但不是上一期持股：續抱 A。
    assert_eq!(rows[1].symbols, ["A"]);
}

/// 設定候選股與續抱名單的產業編號（依序對應）。
fn with_industries(mut row: CandidateRow, candidates: &[i32], hold: &[i32]) -> CandidateRow {
    row.candidate_industries = candidates.to_vec();
    row.hold_industries = hold.to_vec();
    row
}

/// 產業上限：同產業滿 N 檔後排名較後的跳過、由其他產業補上；續抱的股票也計入上限；
/// 沒有產業資料視為同一組（編號 0）。
#[test]
fn select_holdings_caps_stocks_per_industry() {
    // A、B、C 同產業 1，D 產業 2：上限 2 時取 A、B 後跳過 C，補 D。
    let rows = select_holdings(
        vec![with_industries(
            candidates(
                2,
                &[
                    ("A", Some(1.0)),
                    ("B", Some(2.0)),
                    ("C", Some(3.0)),
                    ("D", Some(4.0)),
                ],
            ),
            &[1, 1, 1, 2],
            &[],
        )],
        3,
        3,
        Some(2),
    );
    assert_eq!(rows[0].symbols, ["A", "B", "D"]);
    assert_eq!(rows[0].avg_return, Some(7.0 / 3.0));
    // 不設上限時就是前 3 名。
    let rows = select_holdings(
        vec![with_industries(
            candidates(2, &[("A", None), ("B", None), ("C", None), ("D", None)]),
            &[1, 1, 1, 2],
            &[],
        )],
        3,
        3,
        None,
    );
    assert_eq!(rows[0].symbols, ["A", "B", "C"]);

    // 續抱：上一期 A（產業 1）只符合續抱條件也計入上限，這期產業 1 只能再放 0 檔（上限 1），
    // 候選 B（產業 1）跳過，補 C（產業 3）。
    let rows = select_holdings(
        vec![
            with_industries(candidates(2, &[("A", None)]), &[1], &[]),
            with_hold(
                with_industries(candidates(3, &[("B", None), ("C", None)]), &[1, 3], &[1]),
                &[("A", None)],
            ),
        ],
        2,
        2,
        Some(1),
    );
    assert_eq!(rows[1].symbols, ["C", "A"]);

    // 沒有產業資料（舊資料或測試）視為同一組：上限 1 只放 1 檔。
    let rows = select_holdings(
        vec![candidates(2, &[("A", None), ("B", None)])],
        2,
        2,
        Some(1),
    );
    assert_eq!(rows[0].symbols, ["A"]);
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

/// 持有 20 日以內跑全部偏移；更長時平均抽 20 個（含 0、不超過 hold_days − 1）。
#[test]
fn sample_offsets_covers_or_spreads() {
    use super::offsets::sample_offsets;
    assert_eq!(sample_offsets(5), [0, 1, 2, 3, 4]);
    assert_eq!(sample_offsets(20), (0..20).collect::<Vec<u16>>());
    let sixty = sample_offsets(60);
    assert_eq!(sixty.len(), 20);
    assert_eq!((sixty[0], sixty[1], sixty[19]), (0, 3, 57));
    let long = sample_offsets(240);
    assert_eq!((long.len(), long[1], long[19]), (20, 12, 228));
}

/// 分布：排序後取最小、四分位（線性內插）、中位數、平均與最大；空集合為 None。
#[test]
fn stat_interpolates_quartiles() {
    use super::offsets::stat;
    let result = stat([4.0, 1.0, 3.0, 2.0].into_iter()).expect("有數值");
    assert_eq!(
        (
            result.min,
            result.p25,
            result.median,
            result.mean,
            result.p75,
            result.max
        ),
        (1.0, 1.75, 2.5, 2.5, 3.25, 4.0)
    );
    let single = stat([7.123].into_iter()).expect("一個數值");
    assert_eq!((single.min, single.median, single.max), (7.12, 7.12, 7.12));
    assert!(stat(std::iter::empty()).is_none());
}

/// 各偏移的分布只算有數值的偏移；勝過 0050 的比例只看兩邊都有數值的偏移。
#[test]
fn distribution_counts_offsets_beating_etf() {
    use super::offsets::distribution;
    use crate::interfaces::web::data_api::dto::BacktestOffsetResult;
    let result = |offset: u16, annualized: Option<f64>, sharpe: Option<f64>| {
        let mut summary = summarize(&[], &[], 20);
        summary.annualized_return_after_costs = annualized;
        summary.sharpe_ratio = sharpe;
        summary.max_drawdown_after_costs = annualized.map(|value| -value);
        summary.etf_annualized_return = Some(20.0);
        summary.etf_sharpe_ratio = Some(1.0);
        summary.etf_max_drawdown = Some(-20.0);
        summary.volatility = sharpe.map(|value| value * 10.0);
        summary.etf_volatility = Some(15.0);
        BacktestOffsetResult {
            rebalance_offset: offset,
            summary,
        }
    };
    let results = [
        result(0, Some(30.0), Some(1.5)),
        result(1, Some(10.0), Some(0.5)),
        result(2, Some(25.0), Some(0.9)),
        result(3, None, None),
    ];
    let spread = distribution(&results);
    let annualized = spread.annualized_return_after_costs.expect("有年化報酬");
    assert_eq!(
        (annualized.min, annualized.median, annualized.max),
        (10.0, 25.0, 30.0)
    );
    assert_eq!(
        spread.max_drawdown_after_costs.map(|stat| stat.min),
        Some(-30.0)
    );
    assert_eq!(
        spread.etf_annualized_return.map(|stat| stat.median),
        Some(20.0)
    );
    // 年化 30、25 勝過 20（2/3）；夏普只有 1.5 勝過 1.0（1/3）；回撤只有 -10 比 -20 淺（1/3）。
    assert_eq!(spread.beat_etf_return_rate, Some(0.6667));
    assert_eq!(spread.beat_etf_sharpe_rate, Some(0.3333));
    assert_eq!(spread.beat_etf_drawdown_rate, Some(0.3333));
    assert_eq!(
        spread.volatility.map(|stat| (stat.min, stat.max)),
        Some((5.0, 15.0))
    );
    assert_eq!(spread.etf_volatility.map(|stat| stat.median), Some(15.0));
    assert_eq!(spread.etf_max_drawdown.map(|stat| stat.median), Some(-20.0));

    let empty = distribution(&[]);
    assert!(empty.annualized_return_after_costs.is_none());
    assert!(empty.beat_etf_return_rate.is_none());
    assert!(empty.beat_etf_drawdown_rate.is_none());
}

/// 依年度合併各偏移：每年各自算分布，勝過 0050 的比例只看該年 0050 有資料的偏移；年度由舊到新。
#[test]
fn offset_years_merge_by_year() {
    use super::offsets::offset_years;
    use crate::interfaces::web::data_api::dto::BacktestYear;
    let year = |year: i32, net: f64, etf: Option<f64>| BacktestYear {
        year,
        periods: 1,
        total_return: net,
        total_return_after_costs: net,
        benchmark_total_return: 0.0,
        etf_total_return: etf,
        win_rate: 0.0,
        beat_market_rate: 0.0,
    };
    let years = offset_years(&[
        vec![year(2021, -3.0, Some(2.0)), year(2020, 10.0, Some(5.0))],
        vec![year(2020, 4.0, Some(6.0)), year(2021, 1.0, None)],
        Vec::new(),
    ]);
    assert_eq!(
        years.iter().map(|item| item.year).collect::<Vec<_>>(),
        [2020, 2021]
    );
    assert_eq!(years[0].offsets, 2);
    let net = years[0].return_after_costs.as_ref().expect("有報酬");
    assert_eq!((net.min, net.median, net.max), (4.0, 7.0, 10.0));
    let etf = years[0].etf_return.as_ref().expect("有 0050");
    assert_eq!(etf.median, 5.5);
    assert_eq!(years[0].beat_etf_rate, Some(0.5));
    assert_eq!(years[1].offsets, 2);
    assert_eq!(
        years[1].etf_return.as_ref().map(|stat| stat.median),
        Some(2.0)
    );
    assert_eq!(years[1].beat_etf_rate, Some(0.0));
    let only_missing = offset_years(&[vec![year(2022, 1.0, None)]]);
    assert!(only_missing[0].etf_return.is_none());
    assert!(only_missing[0].beat_etf_rate.is_none());
    assert!(offset_years(&[]).is_empty());
}

/// 0050 前緣：每個比例各偏移混合後彙總；比例 0 為純策略、100 等於只持有 0050；還沒滿持有天數的
/// 最後一期不列入。
#[test]
fn frontier_blends_each_weight() {
    use super::frontier::{WEIGHTS, frontier};
    let rows = vec![
        dated(day(2024, 1, 2), Some(10.0), 0.0, Some(2.0)),
        dated(day(2024, 2, 1), Some(-4.0), 0.0, Some(1.0)),
        dated(day(2024, 3, 1), Some(8.0), 0.0, Some(3.0)),
        PeriodRow {
            pending: true,
            ..dated(day(2024, 4, 1), None, 0.0, None)
        },
    ];
    let raw = Raw {
        costs: free(&rows),
        rows,
        elapsed_days: Some(3),
        fee_discount: 1.0,
    };
    let points = frontier(&[(0, raw.clone()), (1, raw)], 20);
    assert_eq!(
        points
            .iter()
            .map(|point| point.etf_weight)
            .collect::<Vec<_>>(),
        WEIGHTS
    );
    let annualized = |index: usize| {
        points[index]
            .distribution
            .annualized_return_after_costs
            .as_ref()
            .map(|stat| stat.median)
    };
    let etf = points[0]
        .distribution
        .etf_annualized_return
        .as_ref()
        .map(|stat| stat.median);
    assert_eq!(annualized(10), etf);
    assert!(annualized(0) > annualized(10), "{points:?}");
    // 兩個偏移相同，比例越高報酬越接近 0050；全部比例的 0050 欄位都一樣。
    assert!(points.iter().all(|point| {
        point
            .distribution
            .etf_annualized_return
            .as_ref()
            .map(|stat| stat.median)
            == etf
    }));
}

/// 最佳比例：中位數最高者，同值取較低的比例；全部沒有數值為 `None`。
#[test]
fn best_weight_prefers_lower_on_tie() {
    use super::frontier::best_weight;
    use crate::interfaces::web::data_api::dto::{BacktestFrontierPoint, BacktestStat};
    let point = |etf_weight: u8, median: Option<f64>| {
        let mut distribution = super::offsets::distribution(&[]);
        distribution.sharpe_ratio = median.map(|median| BacktestStat {
            min: median,
            p25: median,
            median,
            mean: median,
            p75: median,
            max: median,
        });
        BacktestFrontierPoint {
            etf_weight,
            distribution,
        }
    };
    let sharpe = |spread: &crate::interfaces::web::data_api::dto::BacktestOffsetDistribution| {
        spread.sharpe_ratio.as_ref().map(|stat| stat.median)
    };
    let points = [
        point(0, Some(1.0)),
        point(10, None),
        point(20, Some(1.4)),
        point(30, Some(1.4)),
        point(40, Some(1.2)),
    ];
    assert_eq!(best_weight(&points, sharpe), Some(20));
    assert_eq!(best_weight(&[point(0, None)], sharpe), None);
    assert_eq!(best_weight(&[], sharpe), None);
}

/// 加權方式的參數值雙向對照；不認得的值為 `None`。
#[test]
fn weighting_parses_and_prints() {
    for (name, weighting) in Weighting::ALL {
        assert_eq!(Weighting::parse(name), Some(weighting));
        assert_eq!(weighting.as_str(), name);
    }
    assert_eq!(Weighting::parse("market_cap"), None);
    assert_eq!(Weighting::parse("EQUAL"), None);
}

/// 持股權重：等權各為 1；依成交金額或其平方根；缺值與非正值以最小值補；全部缺值改等權。
#[test]
fn holding_weights_follow_trade_value() {
    use super::portfolio::holding_weights;
    let values = [Some(400.0), None, Some(100.0), Some(0.0)];
    assert_eq!(
        holding_weights(&values, Weighting::Equal),
        [1.0, 1.0, 1.0, 1.0]
    );
    assert_eq!(
        holding_weights(&values, Weighting::TradeValue),
        [400.0, 100.0, 100.0, 100.0]
    );
    assert_eq!(
        holding_weights(&values, Weighting::SqrtTradeValue),
        [20.0, 10.0, 10.0, 10.0]
    );
    assert_eq!(
        holding_weights(&[None, None], Weighting::TradeValue),
        [1.0, 1.0]
    );
    assert!(holding_weights(&[], Weighting::TradeValue).is_empty());
}

/// 依成交金額加權：平均報酬為有報酬持股的加權平均（沒有報酬的不列入）；等權不變；個股貢獻與換手
/// 都依權重計算。
#[test]
fn weigh_uses_trade_value_for_returns_costs_and_contributors() {
    use super::contributors::contributors;
    use super::portfolio::weigh;
    let rows = select_holdings(
        vec![
            CandidateRow {
                candidate_trade_values: vec![Some(300.0), Some(100.0), Some(50.0)],
                ..candidates(2, &[("A", Some(10.0)), ("B", Some(-2.0)), ("C", None)])
            },
            CandidateRow {
                candidate_trade_values: vec![Some(300.0), Some(100.0)],
                ..candidates(3, &[("A", Some(4.0)), ("D", Some(8.0))])
            },
        ],
        3,
        3,
        None,
    );
    assert_eq!(rows[0].trade_values, [Some(300.0), Some(100.0), Some(50.0)]);
    let mut equal = rows.clone();
    weigh(&mut equal, Weighting::Equal);
    assert_eq!(equal, rows);
    assert_eq!(rows[0].avg_return, Some(4.0));

    let mut weighted = rows;
    weigh(&mut weighted, Weighting::TradeValue);
    assert_eq!(weighted[0].weights, [300.0, 100.0, 50.0]);
    // (10×300 − 2×100) ÷ 400 = 7；C 沒有報酬，不列入。
    assert_eq!(weighted[0].avg_return, Some(7.0));
    // (4×300 + 8×100) ÷ 400 = 5。
    assert_eq!(weighted[1].avg_return, Some(5.0));

    // 第一期期末：A 300×1.1、B 100×0.98、C（沒有報酬，視為平均 7%）50×1.07，合計 481.5。
    // 第二期 A 調到 0.75（加碼）、買進 D 0.25，賣出 B、C。
    let costs = trading_costs(&weighted, 0.0);
    assert!(near(costs[0].turnover, 1.0), "{costs:?}");
    let a_before = 330.0 / 481.5;
    assert!(near(costs[1].turnover, 0.75 - a_before + 0.25), "{costs:?}");
    assert!(
        near(
            costs[1].net_return,
            5.0 - (98.0 + 53.5) / 481.5 * super::portfolio::TAX_RATE
        ),
        "{costs:?}"
    );

    let (held, best, worst) = contributors(&weighted, 10);
    assert_eq!(held, 4);
    // A：10×300÷400 + 4×300÷400 = 10.5；D：8×100÷400 = 2；B：−2×100÷400 = −0.5。
    assert_eq!(
        best.iter()
            .map(|item| (item.symbol.as_str(), item.contribution))
            .collect::<Vec<_>>(),
        [("A", 10.5), ("D", 2.0)]
    );
    assert_eq!(
        worst
            .iter()
            .map(|item| (item.symbol.as_str(), item.contribution))
            .collect::<Vec<_>>(),
        [("B", -0.5)]
    );
}
