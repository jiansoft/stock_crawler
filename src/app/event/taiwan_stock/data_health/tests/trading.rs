//! 交易日完整與本益比／淨值比為 0 的檢查。

use super::*;

/// 本益比、淨值比為 0 的比例超過門檻才列出；今天與沒有列數的日子不檢查。
#[test]
fn zero_valuation_ratios_over_the_limit_are_flagged() {
    let zero = |d, market_id, rows, pe_zero, pb_zero| ValuationRatioZeroCount {
        date: day(d),
        market_id,
        rows,
        pe_zero,
        pb_zero,
    };
    let mut snapshot = healthy();
    snapshot.valuation_ratio_zeros = vec![
        zero(8, 2, 1_100, 243, 27),
        zero(8, 4, 894, 236, 2),
        zero(9, 2, 1_100, 243, 1_100),
        zero(9, 4, 894, 890, 2),
        zero(10, 4, 894, 894, 894),
        zero(7, 4, 0, 0, 0),
    ];
    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
    assert_eq!(
        problems_of(&checks, "本益比／淨值比"),
        [
            "2026-10-09 上市 淨值比為 0 的有 1100／1100 檔（請跑 test_backfill_official_valuation_ratios）",
            "2026-10-09 上櫃 本益比為 0 的有 890／894 檔（請跑 test_backfill_official_valuation_ratios）",
        ]
    );
}

/// 平日整天沒有日報價要列出；休市日與週末不算。
#[test]
fn a_missing_weekday_is_flagged_unless_it_is_a_holiday() {
    let mut snapshot = healthy();
    snapshot
        .quoted_dates
        .retain(|date| *date != day(7) && *date != day(8));
    snapshot
        .market_counts
        .retain(|count| count.date != day(7) && count.date != day(8));

    let checks = evaluate(&snapshot, day(4), day(10), Some(&[day(8)]));

    assert_eq!(
        problems_of(&checks, "交易日完整"),
        ["2026-10-07 整天沒有日報價"]
    );
}

/// 只剩零量補值列的市場（有日報價、成交檔數 0）也要列出。
#[test]
fn a_market_with_only_fill_rows_is_flagged() {
    let mut snapshot = healthy();
    snapshot
        .market_counts
        .retain(|count| !(count.date == day(9) && count.market_id == 4));

    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));

    assert_eq!(
        problems_of(&checks, "交易日完整"),
        ["2026-10-09 上櫃 只有 0 檔有成交（近期 880 檔）"]
    );
}

/// 休市日抓不到時不猜哪天該開市，但要在週報註明沒檢查。
#[test]
fn missing_holidays_skip_the_whole_day_check() {
    let mut snapshot = healthy();
    snapshot.quoted_dates.retain(|date| *date != day(7));
    snapshot.market_counts.retain(|count| count.date != day(7));

    let checks = evaluate(&snapshot, day(4), day(10), None);

    assert_eq!(
        problems_of(&checks, "交易日完整"),
        ["休市日清單抓取失敗，未檢查整天缺漏"]
    );
}
