//! public的測試；連外的測試標 #[ignore]。

use super::*;
use crate::infra::cache::SHARE;
use rust_decimal_macros::dec;

/// 以貼近 publicForm 真實回應形狀的資料驗證整列整理流程。
///
/// 涵蓋：民國日期轉西元、承銷價「-」→ None、
/// 以及欄位不足列的防護（略過而不是 panic）。
#[test]
fn map_public_items_maps_rows_and_skips_short_rows() {
    let data = vec![
        // 正常列：12 欄，日期為民國格式。
        vec![
            "1".to_string(),          // 序號
            "115/07/21".to_string(),  // 抽籤日期
            "測試公司".to_string(),   // 證券名稱
            "9999".to_string(),       // 證券代號
            "上市".to_string(),       // 發行市場
            "115/07/14".to_string(),  // 申購開始日
            "115/07/16".to_string(),  // 申購結束日
            "10,000,000".to_string(), // 承銷股數
            "8,000,000".to_string(),  // 實際承銷股數
            "50.00".to_string(),      // 承銷價(元)
            "52.50".to_string(),      // 實際承銷價(元)
            "115/07/28".to_string(),  // 撥券日期
        ],
        // 承銷價尚未公布（「-」）→ offering_price 應為 None。
        vec![
            "2".to_string(),
            "尚未公布".to_string(),
            "另一家公司".to_string(),
            "8888".to_string(),
            "上櫃".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        ],
        // 欄位不足列：直接索引會 panic，必須被防護略過。
        vec!["3".to_string(), "殘缺列".to_string()],
    ];

    let result = map_public_items(data);

    assert_eq!(result.len(), 2, "欄位不足列應被略過");

    let first = &result[0];
    assert_eq!(first.stock_symbol, "9999");
    assert_eq!(first.stock_name, "測試公司");
    assert_eq!(first.market, "上市");
    // 民國 115/07/21 → 西元 2026-07-21。
    assert_eq!(
        first.drawing_date,
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21)
    );
    assert_eq!(
        first.offering_start_date,
        chrono::NaiveDate::from_ymd_opt(2026, 7, 14)
    );
    assert_eq!(
        first.issue_date,
        chrono::NaiveDate::from_ymd_opt(2026, 7, 28)
    );
    // 承銷價取「實際承銷價」欄（index 10）。
    assert_eq!(first.offering_price, Some(dec!(52.50)));

    let second = &result[1];
    assert_eq!(second.stock_symbol, "8888");
    assert_eq!(second.drawing_date, None, "「尚未公布」不是日期 → None");
    assert_eq!(second.offering_price, None, "「-」無法解析 → None");
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 visit");

    match visit().await {
        Ok(list) => {
            //dbg!(&list);
            tracing::debug!("list:{list:#?}");
        }
        Err(why) => {
            tracing::debug!("Failed to visit because: {why:?}");
        }
    }

    tracing::debug!("結束 visit");
}
