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

    let mut result: Vec<EtfInfo> = Vec::with_capacity(512);

    // 組合 API 網址，使用 twse::HOST (twse.com.tw) 避免寫死網域
    let url = format!("https://openapi.{}/v1/opendata/t187ap47_L", twse::HOST);

    // 使用工具函式 get_json 抓取資料並自動轉換為 Vec<TwseEtfRaw>
    let data = util::http::get_json::<Vec<TwseEtfRaw>>(&url).await?;

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
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 單元測試：模擬執行抓取邏輯並列印結果
    #[tokio::test]
    #[ignore]
    async fn test_visit_twse_etf() {
        dotenv::dotenv().ok();

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
