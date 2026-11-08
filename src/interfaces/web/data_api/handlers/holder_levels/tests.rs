//! `/stocks/{symbol}/holder-levels` handler：單一股票單週的集保股權分散全部持股分級的測試。

use super::*;

fn row(level: i16, holders: i64, percent: Decimal) -> LevelRow {
    LevelRow {
        date: NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(),
        level,
        holders,
        shares: holders * 1_000,
        percent,
    }
}

#[test]
fn level_labels_cover_every_tdcc_level() {
    assert_eq!(level_label(1), "1～999 股");
    assert_eq!(level_label(12), "400,001～600,000 股");
    assert_eq!(level_label(15), "1,000,001 股以上");
    assert_eq!(level_label(16), "差異數調整");
    assert_eq!(level_label(17), "合計");
    assert_eq!(level_label(0), "未知級距");
    assert_eq!(level_label(18), "未知級距");
    assert_eq!(level_label(-1), "未知級距");
}

/// 1～15 級依序放進 levels，16 與 17 分開；沒有列時回 `None`。
#[test]
fn build_response_splits_adjustment_and_total() {
    let response = build_response(
        "2330".to_owned(),
        vec![
            row(1, 1_000, Decimal::new(125, 2)),
            row(15, 1_482, Decimal::new(847_800, 4)),
            row(16, 2, Decimal::ZERO),
            row(17, 3_010_913, Decimal::ONE_HUNDRED),
        ],
    )
    .expect("有分級資料");
    assert_eq!(response.date, "2026-10-22");
    assert_eq!(response.levels.len(), 2);
    assert_eq!(response.levels[1].label, "1,000,001 股以上");
    assert_eq!(response.levels[1].percent, Some(84.78));
    assert_eq!(
        response.adjustment.as_ref().map(|item| item.holders),
        Some(2)
    );
    assert_eq!(
        response.total.as_ref().map(|item| item.label.as_str()),
        Some("合計")
    );
    assert_eq!(build_response("2330".to_owned(), Vec::new()), None);
}
