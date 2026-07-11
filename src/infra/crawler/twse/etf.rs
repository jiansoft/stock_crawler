use anyhow::Result;
use chrono::Local;
use serde::Deserialize;

// 匯入專案內部模組：包含共用資訊載體、交易所定義與工具函式
use crate::{
    core::declare::StockExchangeMarket,
    core::util::{self, datetime::Weekend},
    infra::crawler::{share::EtfInfo, twse},
};

/// 證交所 (TWSE) OpenAPI 的原始資料格式
#[derive(Deserialize, Debug)]
struct TwseEtfRaw {
    #[serde(rename = "基金代號")] // 指定 JSON 中的中文欄位對應到 symbol 變數
    pub symbol: String,
    #[serde(rename = "基金簡稱")]
    pub name: String,
    #[serde(rename = "上市日期")]
    pub listing_date: String,
}

/// 調用 TWSE OpenAPI 取得上市市場最新的 ETF 資訊。
pub async fn visit() -> Result<Vec<EtfInfo>> {
    // 週末不處理
    if Local::now().is_weekend() {
        return Ok(Vec::new());
    }

    // 組合 API 網址，使用 twse::HOST (twse.com.tw) 避免寫死網域
    let url = format!("https://openapi.{}/v1/opendata/t187ap47_L", twse::HOST);

    // 使用工具函式 get_json 抓取資料並自動轉換為 Vec<TwseEtfRaw>
    // （fetch/parse 分離：欄位整理交給純函式，讓 fixture 單元測試可以覆蓋）
    let data = util::http::get_json::<Vec<TwseEtfRaw>>(&url).await?;

    Ok(map_etf_items(data))
}

/// 將 TWSE OpenAPI 的原始 ETF 資料整理成專案內部的 [`EtfInfo`]。
///
/// 這是一個「純函式」——輸入只有已反序列化的原始資料，不做任何網路 I/O，
/// 可用 `testdata/etf_t187ap47.json` fixture 直接驗證，涵蓋：
/// 民國短日期（`1150409` → `2026-04-09`）轉換、轉換失敗時保留原始字串、
/// 空日期原樣保留，以及代號與名稱的前後空白清理。
fn map_etf_items(data: Vec<TwseEtfRaw>) -> Vec<EtfInfo> {
    let mut result: Vec<EtfInfo> = Vec::with_capacity(data.len());

    // 取得「上市」市場的定義物件
    let mode = StockExchangeMarket::Listed;

    // 遍歷抓到的每一筆基金資料
    for item in data {
        let industry = "ETF".to_string();
        let industry_id = 9001; // ETF 的固定產業代碼

        // 處理日期：API 給的是 "1150409"，我們要轉成 "2026-04-09"
        let listing_date = if !item.listing_date.is_empty() {
            util::datetime::parse_taiwan_date_short(&item.listing_date)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or(item.listing_date) // 轉失敗就用原始字串
        } else {
            item.listing_date
        };

        // 將整理好的資料推入結果陣列
        result.push(EtfInfo {
            stock_symbol: item.symbol.trim().to_string(),
            name: item.name.trim().to_string(),
            listing_date,
            industry,
            market: mode,
            industry_id,
        });
    }

    // 回傳最終結果
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 以貼近 TWSE OpenAPI 真實回應形狀的 fixture 驗證欄位整理流程。
    ///
    /// fixture 同時驗證 serde 的中文欄位 rename（基金代號/基金簡稱/上市日期）
    /// 與 `map_etf_items` 的整理規則：民國短日期轉西元、
    /// 轉換失敗保留原字串、空日期原樣保留、代號與名稱去前後空白。
    #[test]
    fn map_etf_items_maps_fixture_rows() {
        // include_str! 的路徑相對於本檔案（twse/etf.rs）→ twse/testdata/。
        const FIXTURE: &str = include_str!("testdata/etf_t187ap47.json");

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
}
