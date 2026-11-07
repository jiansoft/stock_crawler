//! listed的測試；連外的測試標 #[ignore]。

use std::result::Result::Ok;

use crate::infra::cache::SHARE;

use super::*;
use rust_decimal_macros::dec;
use serde_json::json;

/// 休市日的回應只有 `stat` 與 `total`，必須能反序列化，才能交給 `stat` 判斷略過。
/// 2026-09-25（中秋節）、2026-09-28（教師節）排程曾因此在解析 JSON 時失敗並發出告警。
#[test]
fn response_on_market_holiday_deserializes_without_data() {
    let body = r#"{"stat":"查詢日期大於可查詢最大日期，請重新查詢!","total":0}"#;

    let response: QFIIResponse = serde_json::from_str(body).expect("休市日回應應可解析");

    assert_eq!(
        response.stat.as_deref(),
        Some("查詢日期大於可查詢最大日期，請重新查詢!")
    );
    assert!(response.data.is_empty());
    assert!(response.select_type.is_empty());
}

/// 以貼近 MI_QFIIS 真實回應形狀的資料驗證資料列整理流程。
///
/// TWSE 的數值欄位都是「含千分位逗號的字串」，
/// 整理時要正確去逗號；欄位數不是 12 的列（表尾統計）要略過。
#[test]
fn map_qfii_rows_maps_rows_and_skips_malformed() {
    let data: Vec<Vec<serde_json::Value>> = vec![
        // 正常列：12 欄。[0]=代號、[3]=發行股數、[5]=外資持股、[7]=持股比率。
        vec![
            json!("2330"),
            json!("台積電"),
            json!("100.00"),
            json!("25,932,070,990"),
            json!("0"),
            json!("19,381,867,412"),
            json!("0"),
            json!("74.74"),
            json!("25.26"),
            json!("100.00"),
            json!("0.00"),
            json!(""),
        ],
        // 欄位數不足的統計列 → 略過。
        vec![json!("合計"), json!("123,456")],
    ];

    let result = map_qfii_rows(data);

    assert_eq!(result.len(), 1, "欄位數不是 12 的列應被略過");
    assert_eq!(result[0].stock_symbol, "2330");
    assert_eq!(result[0].issued_share, 25_932_070_990);
    assert_eq!(result[0].shares_held, 19_381_867_412);
    assert_eq!(result[0].share_holding_percentage, dec!(74.74));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 visit");
    //let date =  DateTime::parse_from_str("2023-09-14 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let datetime_local: DateTime<FixedOffset> =
        match DateTime::parse_from_str("2023-09-15 12:00:00 +0800", "%Y-%m-%d %H:%M:%S %z") {
            Ok(dt) => dt,
            Err(why) => {
                tracing::debug!("error:{:#?}", why);
                return;
            }
        };
    match visit(datetime_local).await {
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
        Ok(qfiis) => {
            tracing::debug!("qfiis:{:#?}", qfiis);
        }
    }
    tracing::debug!("結束 visit");
}
