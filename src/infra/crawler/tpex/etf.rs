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
mod tests;
