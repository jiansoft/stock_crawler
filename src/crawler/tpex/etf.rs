use anyhow::Result;
use chrono::Local;
use serde::Deserialize;

// 匯入專案內部模組
use crate::{
    cache::SHARE,
    crawler::{share::EtfInfo, tpex},
    database::table,
    declare::StockExchangeMarket,
    util::{self, datetime::Weekend},
};

/// 櫃買中心 (TPEx) OpenAPI 的原始資料格式
#[derive(Deserialize, Debug)]
struct TpexEtfRaw {
    #[serde(rename = "SecuritiesCompanyCode")]
    pub code: String,
    #[serde(rename = "CompanyName")]
    pub name: String,
}

/// 調用 TPEx OpenAPI 取得上櫃市場最新的 ETF 資訊。
pub async fn visit() -> Result<Vec<EtfInfo>> {
    // 週末不處理
    if Local::now().is_weekend() {
        return Ok(Vec::new());
    }

    let mut result: Vec<EtfInfo> = Vec::with_capacity(256);

    // 組合 TPEx OpenAPI 網址 (上櫃股票收盤行情)
    let url = format!(
        "https://{}/openapi/v1/tpex_mainboard_daily_close_quotes",
        tpex::HOST
    );

    // 執行 HTTP 請求並解析 JSON
    let data = util::http::get_json::<Vec<TpexEtfRaw>>(&url).await?;

    // 取得「上櫃」市場的定義物件
    let mode = StockExchangeMarket::OverTheCounter;
    let exchange_market = match SHARE.get_exchange_market(mode.serial()) {
        None => table::stock_exchange_market::StockExchangeMarket::new(
            mode.serial(),
            mode.exchange().serial_number(),
        ),
        Some(em) => em,
    };

    for item in data {
        let symbol = item.code.trim();
        let name = item.name.trim();

        // 過濾規則：代號 5 碼以上且 00 開頭，或名稱包含 "基金"/"ETF"
        if (symbol.len() >= 5 && symbol.starts_with("00"))
            || name.contains("基金")
            || name.contains("ETF")
        {
            result.push(EtfInfo {
                stock_symbol: symbol.to_string(),
                name: name.to_string(),
                listing_date: "".to_string(),
                industry: "ETF".to_string(),
                exchange_market: exchange_market.clone(),
                industry_id: 9001,
            });
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit_tpex_etf() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        match visit().await {
            Err(why) => println!("抓取上櫃 ETF 失敗: {:?}", why),
            Ok(result) => {
                println!("找到 {} 檔上櫃 ETF", result.len());
                if !result.is_empty() {
                    println!("範例資料: {:#?}", result[0]);
                }
            }
        }
    }
}
