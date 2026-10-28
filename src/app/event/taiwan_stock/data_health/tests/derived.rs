//! 衍生資料表落後、未來報酬缺漏與月營收到期的檢查。

use super::*;

/// 收盤後的衍生表必須到最新交易日；CAGR、外資持股與籌碼可以晚一天，再晚就算落後；
/// 未來報酬看第 5 個交易日前的基準日。
#[test]
fn derived_tables_must_keep_up_with_the_latest_trading_day() {
    let mut snapshot = healthy();
    for derived in &mut snapshot.derived_tables {
        match derived.table {
            "estimate" => derived.latest = Some(day(8)),
            "stock_cagr" => derived.latest = Some(day(7)),
            "yield_rank" => derived.latest = None,
            "daily_relative_strength" => derived.latest = Some(day(8)),
            "chip_daily" => derived.latest = Some(day(7)),
            "broker_flow" => derived.latest = None,
            "daily_forward_return" => derived.latest = Some(day(1)),
            _ => {}
        }
    }

    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));

    assert_eq!(
        problems_of(&checks, "衍生資料"),
        [
            "estimate 最新 2026-10-08，應至少到 2026-10-09",
            "yield_rank 沒有任何資料",
            "stock_cagr 最新 2026-10-07，應至少到 2026-10-08",
            "daily_relative_strength 最新 2026-10-08，應至少到 2026-10-09",
            "chip_daily 最新 2026-10-07，應至少到 2026-10-08",
            "broker_flow 沒有任何資料",
            "daily_forward_return 最新 2026-10-01，應至少到 2026-10-02",
        ]
    );
}

/// 未來報酬缺漏列出最早的 5 個基準日與總數，沒有缺漏時不列。
#[test]
fn forward_return_gaps_list_the_first_dates() {
    assert!(forward_return_gap_problems(&[]).is_empty());
    let gaps: Vec<ForwardReturnGap> = (1..=7)
        .map(|d| ForwardReturnGap {
            date: NaiveDate::from_ymd_opt(2022, 10, d).expect("日期應合法"),
            rows: 2_000 + i64::from(d),
        })
        .collect();
    let problems = forward_return_gap_problems(&gaps);
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with(
            "7 個基準日（2022-10-01～2022-10-07）的短天期報酬缺漏：2022-10-01 2001 列、"
        ),
        "{problems:?}"
    );
    assert!(
        problems[0].ends_with(
            "2022-10-05 2005 列…；請以 MANUAL_FORWARD_RETURN_FROM=2022-10-01 重跑 test_backfill_forward_returns"
        ),
        "{problems:?}"
    );
    let single = forward_return_gap_problems(&gaps[..1]);
    assert!(single[0].contains("：2022-10-01 2001 列；"), "{single:?}");
}

/// 15 日之前不要求上個月的營收；之後就要有。
#[test]
fn revenue_is_due_after_the_fifteenth() {
    assert!(revenue_problems(Some(202608), day(15)).is_empty());
    assert_eq!(
        revenue_problems(Some(202608), day(16)),
        ["最新只到 202608，應已有 202609"]
    );
    assert!(revenue_problems(Some(202609), day(16)).is_empty());
    // 一月要看前一年十二月。
    let january = NaiveDate::from_ymd_opt(2027, 1, 20).expect("日期應合法");
    assert_eq!(
        revenue_problems(Some(202611), january),
        ["最新只到 202611，應已有 202612"]
    );
}
