use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

/// 每季一期、報酬依序取 `returns`（扣成本後相同），0050 固定 1%。
fn track(start: i32, returns: &[f64]) -> Track {
    let rows: Vec<PeriodRow> = returns
        .iter()
        .enumerate()
        .map(|(index, ret)| PeriodRow {
            date: day(start + index as i32 / 4, 1 + 3 * (index as u32 % 4), 2),
            holdings: 1,
            avg_return: Some(*ret),
            etf_return: Some(1.0),
            ..PeriodRow::default()
        })
        .collect();
    let costs = returns
        .iter()
        .map(|ret| PeriodCost {
            turnover: 0.0,
            net_return: *ret,
        })
        .collect();
    Track { rows, costs }
}

/// 緩衝依 limit 的 1、1.5、2 倍且不超過 200、去除重複；組合數為 3 × 3 × 4，順序先緩衝再上限再
/// 0050 比例。
#[test]
fn candidate_grid_covers_combinations() {
    assert_eq!(keep_ranks(20), [20, 30, 40]);
    assert_eq!(keep_ranks(150), [150, 200]);
    assert_eq!(keep_ranks(1), [1, 2]);
    let grid = candidates(20);
    assert_eq!(grid.len(), 36);
    assert_eq!(
        grid[0],
        Candidate {
            etf_weight: 0,
            max_weight: None,
            keep_rank: 20
        }
    );
    assert_eq!(grid[1].etf_weight, 30);
    assert_eq!(grid[4].max_weight, Some(10));
    assert_eq!(grid[12].keep_rank, 30);
}

/// 每年用前兩年挑夏普值最高的組合套用到這一年；訓練期間不足 2 期（沒有夏普值）的年份略過；
/// 串起來的期數只含樣本外年度，並附不調參數與 0050 的同期報酬。
#[test]
fn walk_forward_picks_best_training_sharpe_per_year() {
    let grid = [
        Candidate {
            etf_weight: 0,
            max_weight: None,
            keep_rank: 20,
        },
        Candidate {
            etf_weight: 50,
            max_weight: Some(10),
            keep_rank: 30,
        },
    ];
    // 2020～2021 第一組波動大、第二組穩定；2022 起第一組變穩定、第二組變差。
    let steady = [2.0, 3.0, 2.0, 3.0];
    let swing = [10.0, -8.0, 9.0, -7.0];
    let first: Vec<f64> = [swing, swing, steady, steady, steady].concat();
    let second: Vec<f64> = [steady, steady, swing, [-5.0, -6.0, -5.0, -6.0], swing].concat();
    let tracks = [track(2020, &first), track(2020, &second)];
    let baseline = track(2020, &[1.0; 20]);
    let result = walk_forward(
        &grid,
        &tracks,
        &baseline,
        day(2020, 1, 2),
        day(2024, 12, 31),
        2,
        60,
    );
    let years: Vec<(i32, u8)> = result
        .years
        .iter()
        .map(|year| (year.year, year.etf_weight))
        .collect();
    // 2022：看 2020～2021 選第二組；2023：看 2021～2022，第二組的穩定被 2022 拉下，選第一組。
    assert_eq!(years, [(2022, 50), (2023, 0), (2024, 0)]);
    let first_year = &result.years[0];
    assert_eq!(first_year.max_weight, Some(10));
    assert_eq!(first_year.keep_rank, 30);
    assert_eq!(first_year.periods, 4);
    let swing_return = (1.10 * 0.92 * 1.09 * 0.93 - 1.0) * 100.0;
    assert_eq!(first_year.return_after_costs, round2(swing_return));
    assert_eq!(
        first_year.baseline_return_after_costs,
        round2((1.01_f64.powi(4) - 1.0) * 100.0)
    );
    assert_eq!(
        first_year.etf_return,
        first_year.baseline_return_after_costs.into()
    );
    assert_eq!(result.chosen.rows.len(), 12);
    assert_eq!(result.chosen.costs.len(), 12);
    assert_eq!(result.baseline.rows.len(), 12);
    assert_eq!(result.chosen.rows[0].date, day(2022, 1, 2));

    // 訓練期間只有 1 期：沒有夏普值，全部略過。
    let short = [track(2020, &[1.0]), track(2020, &[2.0])];
    let result = walk_forward(
        &grid,
        &short,
        &baseline,
        day(2020, 1, 2),
        day(2021, 12, 31),
        1,
        60,
    );
    assert!(result.years.is_empty());
    assert!(result.chosen.rows.is_empty());
}

/// 滾動前推訓練期間排除跨越年度邊界的期數（B03）：12 月換股跨入 1 月的價格不洩漏給當期訓練。
#[test]
fn walk_forward_train_range_excludes_cross_boundary_december_period() {
    let rows = vec![
        PeriodRow {
            date: day(2023, 11, 1),
            exit_date: Some(day(2023, 11, 29)),
            holdings: 1,
            avg_return: Some(5.0),
            ..PeriodRow::default()
        },
        PeriodRow {
            date: day(2023, 12, 15),
            exit_date: Some(day(2024, 1, 15)),
            holdings: 1,
            avg_return: Some(20.0),
            ..PeriodRow::default()
        },
    ];
    let costs = vec![
        PeriodCost {
            turnover: 0.0,
            net_return: 5.0,
        },
        PeriodCost {
            turnover: 0.0,
            net_return: 20.0,
        },
    ];
    let track = Track {
        rows: rows.clone(),
        costs: costs.clone(),
    };
    // 訓練期間為 2023 全年（2023-01-01 至 2024-01-01）：
    // 12-15 換股日雖然在 2023 年，但出場日為 2024-01-15，含有 2024 年價格，必須排除在 2023 訓練之外。
    let train_range = track.train_range(day(2023, 1, 1), day(2024, 1, 1));
    assert_eq!(train_range, 0..1);

    // 修改跨年期數的報酬（模擬 1 月暴跌），不影響訓練期的夏普值。
    let mut modified_rows = rows;
    modified_rows[1].avg_return = Some(-50.0);
    let mut modified_costs = costs;
    modified_costs[1].net_return = -50.0;
    let modified_track = Track {
        rows: modified_rows,
        costs: modified_costs,
    };
    assert_eq!(
        track.sharpe(train_range.clone(), 20),
        modified_track.sharpe(
            modified_track.train_range(day(2023, 1, 1), day(2024, 1, 1)),
            20
        )
    );
}
