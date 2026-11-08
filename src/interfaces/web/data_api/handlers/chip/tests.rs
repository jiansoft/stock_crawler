//! `/stocks/{symbol}/chip` handler：單一股票的籌碼面的測試。

use super::*;

/// 連續天數只算到第一個缺資料、0 或反向為止。
#[test]
fn streak_counts_same_direction_from_the_latest_day() {
    assert_eq!(streak([Some(5), Some(1), Some(-2), Some(3)].into_iter()), 2);
    assert_eq!(streak([Some(-5), Some(-1), Some(-2)].into_iter()), -3);
    assert_eq!(streak([Some(5), None, Some(3)].into_iter()), 1);
    assert_eq!(streak([Some(0), Some(3)].into_iter()), 0);
    assert_eq!(streak([None, Some(3)].into_iter()), 0);
    assert_eq!(streak(std::iter::empty()), 0);
}

/// 超過明細最大筆數的連續期間仍須完整計算，直到零或缺值才停止。
#[test]
fn streak_is_not_capped_by_the_detail_window() {
    assert_eq!(
        streak(std::iter::repeat_n(Some(1), 130).chain([Some(0), Some(1)])),
        130
    );
    assert_eq!(
        streak(std::iter::repeat_n(Some(-1), 130).chain([None, Some(-1)])),
        -130
    );
}

/// 融資融券增減由今日減前日；法人任一類缺資料時合計為 null。
#[test]
fn daily_row_computes_changes_and_totals() {
    let row = ChipDailyRow {
        date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        foreign_net: Some(-1_672_231),
        trust_net: Some(154_586),
        dealer_net: Some(395_975),
        margin_previous: Some(30_135),
        margin_balance: Some(30_500),
        short_previous: None,
        short_balance: Some(49),
        main_net: Some(-120),
        main_share: Some(Decimal::new(-35, 1)),
    };
    let day = row.into_dto();
    assert_eq!(day.total_net, Some(-1_121_670));
    assert_eq!(day.margin_change, Some(365));
    assert_eq!(day.short_change, None);
    assert_eq!(day.main_net, Some(-120));
    assert_eq!(day.main_share, Some(-3.5));
}

/// 分點的比重存成字串也要讀得出來；欄位不全的項目略過。
#[test]
fn parse_brokers_reads_string_shares() {
    let brokers = parse_brokers(
        r#"[{"name":"凱基-台北","buy":150,"sell":30,"net":120,"share":"6.98"},{"name":"缺欄位"}]"#,
    );
    assert_eq!(brokers.len(), 1);
    assert_eq!(brokers[0].name, "凱基-台北");
    assert_eq!(brokers[0].share, Some(6.98));
    assert!(parse_brokers("not json").is_empty());
}

/// 設質比例以設質 ÷ 持股計算，持股為 0 時為 null。
#[test]
fn insider_summary_computes_pledge_percent() {
    let row = |shares: i64, pledged: i64| InsiderRow {
        month: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
        insiders: 18,
        shares: Decimal::from(shares),
        pledged: Decimal::from(pledged),
        related_pledged: Decimal::ZERO,
    };
    let summary = insider_summary(row(1_743_969_104, 860_000_000));
    assert_eq!(summary.month, "2026-08");
    assert_eq!(summary.pledge_percent, Some(49.31));
    assert_eq!(insider_summary(row(0, 0)).pledge_percent, None);
}
