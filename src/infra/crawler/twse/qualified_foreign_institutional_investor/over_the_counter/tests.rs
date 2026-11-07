//! over_the_counter的測試；連外的測試標 #[ignore]。

use std::result::Result::Ok;

use crate::infra::cache::SHARE;

use super::*;
use rust_decimal_macros::dec;

fn parse(json: &str) -> Result<OtcQfiiSnapshot> {
    parse_tpex_qfii(serde_json::from_str(json).expect("fixture should be valid JSON"))
}

/// 以 2026-09-30 真實回應截取的 fixture 驗證欄位定位、日期與數字清理（千分位逗號、`%`）。
#[test]
fn parse_tpex_qfii_parses_fixture_rows() {
    // include_str! 的路徑相對於本檔案 → 上一層的 testdata/。
    const FIXTURE: &str = include_str!("../testdata/qfii_otc.json");

    let snapshot = parse(FIXTURE).unwrap();

    assert_eq!(snapshot.date, NaiveDate::from_ymd_opt(2026, 9, 30).unwrap());
    assert_eq!(snapshot.rows.len(), 3);
    let yuan_tai = snapshot
        .rows
        .iter()
        .find(|dto| dto.stock_symbol == "8069")
        .unwrap();
    assert_eq!(yuan_tai.issued_share, 1_154_360_555);
    assert_eq!(yuan_tai.shares_held, 463_977_020);
    assert_eq!(yuan_tai.share_holding_percentage, dec!(40.19));

    let gw = snapshot
        .rows
        .iter()
        .find(|dto| dto.stock_symbol == "6488")
        .unwrap();
    assert_eq!(gw.share_holding_percentage, dec!(26.37));
}

/// 欄位順序調整時仍依名稱取值。
#[test]
fn parse_tpex_qfii_locates_columns_by_name() {
    let json = r#"{"stat":"ok","date":"20260930","tables":[{
        "fields":["僑外資及陸資持股比率(E=C/A)","僑外資及陸資持有股數(C)","發行股數(A)","代號"],
        "data":[["10.5%","1,000","10,000","1234"]]}]}"#;

    let rows = parse(json).unwrap().rows;

    assert_eq!(rows[0].stock_symbol, "1234");
    assert_eq!(rows[0].issued_share, 10_000);
    assert_eq!(rows[0].shares_held, 1_000);
    assert_eq!(rows[0].share_holding_percentage, dec!(10.5));
}

/// 休市日（指定日期回補時）櫃買回 ok 但沒有資料列：解析成空清單，由呼叫端略過。
#[test]
fn parse_tpex_qfii_returns_empty_rows_on_market_holiday() {
    let snapshot =
        parse(r#"{"stat":"ok","date":"20260829","tables":[{"fields":["代號"],"data":[]}]}"#)
            .unwrap();

    assert_eq!(snapshot.date, NaiveDate::from_ymd_opt(2026, 8, 29).unwrap());
    assert!(snapshot.rows.is_empty());
}

/// 每日排程不允許空結果；stat 異常、日期缺失、缺必要欄位都要回錯誤，不能默默成功
/// （舊版 MOPS 頁 404 時就是這樣靜默失敗）。
#[test]
fn parse_tpex_qfii_rejects_unexpected_responses() {
    let empty = parse(r#"{"stat":"ok","date":"20260930","tables":[]}"#).unwrap();
    assert!(ensure_not_empty(&empty).is_err());

    assert!(parse(r#"{"stat":"error","date":"20260930","tables":[]}"#).is_err());
    assert!(parse(r#"{"stat":"ok","tables":[]}"#).is_err());

    let missing_column = r#"{"stat":"ok","date":"20260930","tables":[{"fields":["代號","名稱"],"data":[["1234","測試"]]}]}"#;
    let err = parse(missing_column).expect_err("missing column should be an error");
    assert!(err.to_string().contains("發行股數"));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 visit");

    match visit().await {
        Ok(snapshot) => {
            println!(
                "上櫃 QFII {} 共 {} 筆，第一筆：{:?}",
                snapshot.date,
                snapshot.rows.len(),
                snapshot.rows.first()
            );
        }
        Err(why) => {
            println!("Failed to visit because: {:?}", why);
        }
    }
    tracing::debug!("結束 visit");
}
