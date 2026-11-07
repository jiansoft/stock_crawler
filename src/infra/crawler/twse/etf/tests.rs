//! etf的測試；連外的測試標 #[ignore]。

use super::*;

/// 以貼近 TWSE OpenAPI 真實回應形狀的 fixture 驗證欄位整理流程。
///
/// fixture 同時驗證 serde 的中文欄位 rename（基金代號/基金簡稱/上市日期）
/// 與 `map_etf_items` 的整理規則：民國短日期轉西元、
/// 轉換失敗保留原字串、空日期原樣保留、代號與名稱去前後空白。
#[test]
fn map_etf_items_maps_fixture_rows() {
    // include_str! 的路徑相對於本檔案（twse/etf/tests.rs）→ twse/testdata/。
    const FIXTURE: &str = include_str!("../testdata/etf_t187ap47.json");

    let raw: Vec<TwseEtfRaw> = serde_json::from_str(FIXTURE).unwrap();
    let result = map_etf_items(raw);

    assert_eq!(result.len(), 4);

    // 民國短日期 0920630 → 2003-06-30（ROC 92 年）。
    assert_eq!(result[0].stock_symbol, "0050");
    assert_eq!(result[0].name, "元大台灣50");
    assert_eq!(result[0].listing_date, "2003-06-30");
    assert_eq!(result[0].market, StockExchangeMarket::Listed);
    assert_eq!(result[0].industry, "ETF");
    assert_eq!(result[0].industry_id, 9001);

    // 民國短日期 1140513 → 2025-05-13（ROC 114 年）。
    assert_eq!(result[1].listing_date, "2025-05-13");

    // 代號與名稱的前後空白要清掉；空日期原樣保留（不猜日期）。
    assert_eq!(result[2].stock_symbol, "0056");
    assert_eq!(result[2].name, "元大高股息");
    assert_eq!(result[2].listing_date, "");

    // 無法解析的日期保留原始字串，交由下游決定如何處理。
    assert_eq!(result[3].listing_date, "not-a-date");
}

/// 單元測試：模擬執行抓取邏輯並列印結果
#[tokio::test]
#[ignore]
async fn test_visit_twse_etf() {
    dotenvy::dotenv().ok();

    match visit().await {
        Err(why) => println!("抓取上市 ETF 失敗: {:?}", why),
        Ok(result) => {
            println!("找到 {} 檔上市 ETF", result.len());
            if !result.is_empty() {
                println!("範例資料: {:#?}", result[0]);
            }
        }
    }
}
