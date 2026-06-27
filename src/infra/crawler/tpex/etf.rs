use anyhow::Result;
use chrono::Local;
use serde::Deserialize;

// 匯入專案內部模組
use crate::{
    core::declare::StockExchangeMarket,
    core::util::{self, datetime::Weekend},
    infra::crawler::{share::EtfInfo, tpex},
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

    let url = format!(
        "https://{}/openapi/v1/tpex_mainboard_daily_close_quotes",
        tpex::HOST
    );

    let data = util::http::get_json::<Vec<TpexEtfRaw>>(&url).await?;
    Ok(parse_etf_items(data, StockExchangeMarket::OverTheCounter))
}

/// 從 TPEx OpenAPI 原始資料中篩選 ETF 項目並轉換為 `EtfInfo`。
///
/// 過濾規則：代號 5 碼以上且以 `00` 開頭，或名稱含「基金」/「ETF」。
fn parse_etf_items(data: Vec<TpexEtfRaw>, mode: StockExchangeMarket) -> Vec<EtfInfo> {
    data.into_iter()
        .filter(|item| {
            let symbol = item.code.trim();
            let name = item.name.trim();
            (symbol.len() >= 5 && symbol.starts_with("00"))
                || name.contains("基金")
                || name.contains("ETF")
        })
        .map(|item| EtfInfo {
            stock_symbol: item.code.trim().to_string(),
            name: item.name.trim().to_string(),
            listing_date: "".to_string(),
            industry: "ETF".to_string(),
            market: mode,
            industry_id: 9001,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw(code: &str, name: &str) -> TpexEtfRaw {
        TpexEtfRaw {
            code: code.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn test_parse_etf_items_includes_00_prefix_symbols() {
        let data = vec![
            make_raw("00878", "國泰永續高股息"),
            make_raw("00692", "富邦公司治理"),
            make_raw("2330", "台積電"),
        ];
        let result = parse_etf_items(data, StockExchangeMarket::OverTheCounter);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].stock_symbol, "00878");
        assert_eq!(result[1].stock_symbol, "00692");
    }

    #[test]
    fn test_parse_etf_items_includes_name_with_etf_or_fund() {
        let data = vec![
            make_raw("9999X", "某某ETF"),
            make_raw("8888X", "某某基金"),
            make_raw("7777A", "普通股票"),
        ];
        let result = parse_etf_items(data, StockExchangeMarket::OverTheCounter);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|e| e.name == "某某ETF"));
        assert!(result.iter().any(|e| e.name == "某某基金"));
    }

    #[test]
    fn test_parse_etf_items_excludes_regular_stocks() {
        let data = vec![make_raw("2330", "台積電"), make_raw("6609", "唯亞威")];
        let result = parse_etf_items(data, StockExchangeMarket::OverTheCounter);
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit_tpex_etf() {
        dotenv::dotenv().ok();

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
