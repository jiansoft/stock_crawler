//! 條件選股回測的換股日偏移穩健度與 0050 核心部位前緣測試。

use super::*;

/// 持有 20 日以內跑全部偏移；更長時平均抽 20 個（含 0、不超過 hold_days − 1）。
#[test]
fn sample_offsets_covers_or_spreads() {
    use super::super::offsets::sample_offsets;
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
    use super::super::offsets::stat;
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
    use super::super::offsets::distribution;
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
    use super::super::offsets::offset_years;
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
    use super::super::frontier::{WEIGHTS, frontier};
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
    let raws = [(0, raw.clone()), (1, raw)];
    let points = frontier(&raws, 20, None);
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
    assert!(
        points
            .iter()
            .all(|point| point.before_split.is_none() && point.after_split.is_none())
    );

    // 分割日 02-01：01-02 那期為樣本內，02-01、03-01 為樣本外（還沒滿持有天數的 04-01 不列入）。
    let points = frontier(&raws, 20, Some(day(2024, 2, 1)));
    let periods =
        |spread: &Option<crate::interfaces::web::data_api::dto::BacktestOffsetDistribution>| {
            spread
                .as_ref()
                .and_then(|spread| spread.etf_annualized_return.as_ref())
                .map(|stat| stat.median)
        };
    assert!(periods(&points[10].before_split).is_some(), "{points:?}");
    assert_ne!(
        periods(&points[10].before_split),
        periods(&points[10].after_split)
    );
    assert_eq!(
        points[0]
            .after_split
            .as_ref()
            .and_then(|spread| spread.annualized_return_after_costs.as_ref())
            .map(|stat| stat.median),
        points[0]
            .after_split
            .as_ref()
            .and_then(|spread| spread.annualized_return_after_costs.as_ref())
            .map(|stat| stat.max),
        "兩個偏移相同，分布只有一個值"
    );
}

/// 樣本內外：依換股日切開各自彙總；沒有分割日時都是 `None`。
#[test]
fn split_results_partition_by_rebalance_date() {
    use super::super::offsets::{split_distributions, split_results};
    use super::super::summary::summarize;
    let rows = vec![
        dated(day(2024, 1, 2), Some(10.0), 0.0, Some(2.0)),
        dated(day(2024, 2, 1), Some(-4.0), 0.0, Some(1.0)),
        dated(day(2024, 3, 1), Some(8.0), 0.0, Some(3.0)),
    ];
    let costs = free(&rows);
    assert!(split_results(3, &rows, &costs, 20, None).is_none());
    let (before, after) =
        split_results(3, &rows, &costs, 20, Some(day(2024, 2, 1))).expect("有分割日");
    assert_eq!((before.rebalance_offset, after.rebalance_offset), (3, 3));
    assert_eq!(before.summary, summarize(&rows[..1], &costs[..1], 20));
    assert_eq!(after.summary, summarize(&rows[1..], &costs[1..], 20));
    // 分割日早於全部換股日：樣本內沒有期數。
    let (empty, all) =
        split_results(0, &rows, &costs, 20, Some(day(2023, 1, 1))).expect("有分割日");
    assert_eq!(empty.summary.periods, 0);
    assert_eq!(all.summary, summarize(&rows, &costs, 20));

    assert_eq!(split_distributions(Vec::new()), (None, None));
    let (before, after) = split_distributions(vec![(before, after)]);
    assert!(before.is_some() && after.is_some());
}

/// 最佳比例：中位數最高者，同值取較低的比例；全部沒有數值為 `None`。
#[test]
fn best_weight_prefers_lower_on_tie() {
    use super::super::frontier::best_weight;
    use crate::interfaces::web::data_api::dto::{BacktestFrontierPoint, BacktestStat};
    let point = |etf_weight: u8, median: Option<f64>| {
        let mut distribution = super::super::offsets::distribution(&[]);
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
            before_split: None,
            after_split: None,
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

/// 樣本內外分割時排除跨越分割日的期數（B03）：換股日在分割日前但出場日跨越分割日的期數不計入樣本內。
#[test]
fn split_results_excludes_cross_boundary_periods() {
    use super::super::offsets::split_results;
    use super::super::summary::summarize;
    let mut rows = vec![
        dated(day(2023, 11, 1), Some(5.0), 0.0, Some(1.0)),
        dated(day(2023, 12, 15), Some(12.0), 0.0, Some(2.0)),
        dated(day(2024, 1, 15), Some(3.0), 0.0, Some(1.0)),
    ];
    rows[0].exit_date = Some(day(2023, 11, 29));
    // 跨年跨界期：換股日 2023-12-15 早於分割日 2024-01-01，但出場日 2024-01-15 跨入隔年。
    rows[1].exit_date = Some(day(2024, 1, 15));
    rows[2].exit_date = Some(day(2024, 2, 15));
    let costs = free(&rows);
    let (before, after) =
        split_results(0, &rows, &costs, 20, Some(day(2024, 1, 1))).expect("有分割日");
    // 樣本內只含 11-01 該期，12-15 跨界期數被排除，不計入樣本內。
    assert_eq!(before.summary.periods, 1);
    assert_eq!(before.summary, summarize(&rows[..1], &costs[..1], 20));
    // 樣本外從 01-15 開始。
    assert_eq!(after.summary.periods, 1);
    assert_eq!(after.summary, summarize(&rows[2..], &costs[2..], 20));
}
