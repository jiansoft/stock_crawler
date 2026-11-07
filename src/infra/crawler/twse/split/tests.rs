//! 上市股票變更面額與 ETF 分割（反分割）恢復買賣參考價格的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

const PAR_VALUE_FIXTURE: &str = include_str!("../testdata/par_value_change_twtb8u.json");
const ETF_SPLIT_FIXTURE: &str = include_str!("../testdata/etf_split_twtcau.json");

fn parse_fixture(fixture: &str, layout: &Layout) -> Vec<CorporateAction> {
    let response: SplitResponse = serde_json::from_str(fixture).expect("fixture 是合法 JSON");
    parse_rows(response.data.unwrap_or_default(), layout)
}

fn find<'a>(actions: &'a [CorporateAction], symbol: &str) -> &'a CorporateAction {
    actions
        .iter()
        .find(|action| action.stock_symbol == symbol)
        .unwrap_or_else(|| panic!("找不到 {symbol}"))
}

/// 面額變更 10 筆全部是分割，比例為整數倍。
#[test]
fn parse_par_value_changes_fixture() {
    let actions = parse_fixture(PAR_VALUE_FIXTURE, &PAR_VALUE_CHANGE);
    assert_eq!(actions.len(), 10);
    assert!(
        actions
            .iter()
            .all(|action| action.action_type == CorporateActionType::Split)
    );

    let chang_wah = find(&actions, "8070");
    assert_eq!(
        chang_wah.effective_date,
        NaiveDate::from_ymd_opt(2020, 8, 17).unwrap()
    );
    assert_eq!(chang_wah.share_ratio, dec!(10));
    assert_eq!(chang_wah.note, "變更面額（分割）：190 ÷ 19");
    assert_eq!(find(&actions, "6949").share_ratio, dec!(20));
    assert_eq!(find(&actions, "6415").share_ratio, dec!(4));
}

/// ETF 分割與反分割：比例對齊到整數倍，修正舊版推錯的 00663L（7）與 00685L（24）。
#[test]
fn parse_etf_splits_fixture() {
    let actions = parse_fixture(ETF_SPLIT_FIXTURE, &ETF_SPLIT);
    assert_eq!(actions.len(), 11);

    assert_eq!(find(&actions, "0052").share_ratio, dec!(7));
    assert_eq!(find(&actions, "00631L").share_ratio, dec!(22));
    assert_eq!(find(&actions, "00663L").share_ratio, dec!(7));
    assert_eq!(find(&actions, "00685L").share_ratio, dec!(24));

    let reverse = find(&actions, "00632R");
    assert_eq!(reverse.action_type, CorporateActionType::ReverseSplit);
    assert_eq!(reverse.share_ratio, dec!(0.14285714));
    assert_eq!(reverse.note, "ETF（反分割）：3.28 ÷ 22.96");
    assert_eq!(find(&actions, "00676R").share_ratio, dec!(0.16666667));
}

/// 1% 以內對齊整數倍；超出容許誤差（不是整數倍的事件）保留原比例。
#[test]
fn snap_ratio_aligns_only_near_whole_multiples() {
    assert_eq!(snap_ratio(dec!(7.0006)), dec!(7));
    assert_eq!(snap_ratio(dec!(0.25)), dec!(0.25));
    assert_eq!(snap_ratio(dec!(0.1668)), dec!(0.16666667));
    assert_eq!(snap_ratio(dec!(2.5)), dec!(2.5));
    assert_eq!(snap_ratio(dec!(0.4)), dec!(0.4));
}

/// 價格是 `-`（尚未恢復交易）或欄位不足的列略過，不當成 0。
#[test]
fn parse_rows_skips_unpriced_and_short_rows() {
    let row = |close: &str, reference: &str| -> Vec<String> {
        ["115/10/20", "1234", "測試", close, reference]
            .into_iter()
            .map(str::to_string)
            .collect()
    };
    let actions = parse_rows(
        vec![
            row("-", "-"),
            row("100", "0"),
            vec!["115/10/20".to_string()],
            row("100", "10"),
        ],
        &PAR_VALUE_CHANGE,
    );
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].share_ratio, dec!(10));
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    let start = NaiveDate::from_ymd_opt(2011, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
    let par = visit_par_value_changes(start, end).await.expect("面額變更");
    let etf = visit_etf_splits(start, end).await.expect("ETF 分割");
    dbg!(par.len(), etf.len());
    assert!(!par.is_empty() && !etf.is_empty());
}
