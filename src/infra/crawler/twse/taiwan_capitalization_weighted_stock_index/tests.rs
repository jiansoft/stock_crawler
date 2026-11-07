//! taiwan_capitalization_weighted_stock_index的測試；連外的測試標 #[ignore]。

use std::result::Result::Ok;

use crate::infra::cache::SHARE;

use super::*;

/// 驗證 FMTQIK API 回應能正確反序列化成 DTO。
///
/// 這個爬蟲沒有額外的解析邏輯（visit 只是 get_json），
/// 所以測試重點是 serde 欄位對應：正常回應與「查無資料」
/// （TWSE 以 stat 表達，data 缺席）兩種形狀都不能失敗。
#[test]
fn taiwan_stock_index_dto_deserializes_both_shapes() {
    // 正常回應：stat=OK，data 是字串矩陣（日期為民國格式）。
    let ok_body = r#"{
        "stat": "OK",
        "date": "20260710",
        "title": "115年07月 市場成交資訊",
        "fields": ["日期", "成交股數", "成交金額", "成交筆數", "發行量加權股價指數", "漲跌點數"],
        "data": [
            ["115/07/09", "8,431,973,318", "512,712,999,451", "3,405,614", "28,553.53", "120.57"],
            ["115/07/10", "7,921,553,101", "488,210,357,822", "3,211,807", "28,439.11", "-114.42"]
        ]
    }"#;
    let dto: TaiwanStockIndexDto = serde_json::from_str(ok_body).unwrap();
    assert_eq!(dto.stat, "OK");
    let data = dto.data.unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0][4], "28,553.53");

    // 查無資料：只有 stat，其他欄位缺席 → Option 欄位須為 None，不得反序列化失敗。
    let empty_body = r#"{ "stat": "很抱歉，沒有符合條件的資料!" }"#;
    let dto: TaiwanStockIndexDto = serde_json::from_str(empty_body).unwrap();
    assert_ne!(dto.stat, "OK");
    assert!(dto.data.is_none());
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 visit");

    match visit(Local::now()).await {
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
        Ok(list) => {
            tracing::debug!("data:{:#?}", list);
        }
    }
    tracing::debug!("結束 visit");
}
