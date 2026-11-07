//! # 上櫃個股日本益比與股價淨值比
//!
//! 來源：TPEx `www/zh-tw/afterTrading/peQryDate?date=YYYY/MM/DD&response=json`
//! （上櫃股票個股本益比、殖利率、股價淨值比依日期查詢），可查任意歷史日期。
//! 每日收盤流程用的 openapi `tpex_mainboard_peratio_analysis` 只給最新一天，
//! 所以上櫃的歷史本益比要從這裡回補。
//!
//! 回應是 `{"stat":"ok","tables":[{"fields":[…],"data":[[…]]}]}`；代號欄叫「股票代號」，
//! 公司名稱尾端有全形空白補齊，沒有值時是 `N/A`。非交易日 `data` 為空。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util,
    domain::quote::official_ratio::{OfficialRatio, parse_ratio},
    infra::crawler::tpex,
};

#[derive(Debug, Default, Deserialize)]
struct PeQryResponse {
    #[serde(default)]
    tables: Vec<PeQryTable>,
}

#[derive(Debug, Default, Deserialize)]
struct PeQryTable {
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得指定日期的上櫃個股本益比與股價淨值比；非交易日回傳空陣列。
///
/// # Errors
///
/// 請求失敗、回應不是預期的 JSON，或有資料卻缺少「股票代號」欄位時回傳錯誤。
pub async fn visit(date: NaiveDate) -> Result<Vec<OfficialRatio>> {
    let url = format!(
        "https://{host}/www/zh-tw/afterTrading/peQryDate?date={date}&response=json",
        host = tpex::HOST,
        date = date.format("%Y%%2F%m%%2F%d")
    );
    let response = util::http::get_json::<PeQryResponse>(&url).await?;
    parse(response)
}

/// 解析 peQryDate 回應。
fn parse(response: PeQryResponse) -> Result<Vec<OfficialRatio>> {
    let Some(table) = response.tables.into_iter().next() else {
        return Ok(Vec::new());
    };
    if table.data.is_empty() {
        return Ok(Vec::new());
    }
    let column = |name: &str| table.fields.iter().position(|field| field == name);
    let symbol = column("股票代號").ok_or_else(|| anyhow!("peQryDate 缺少「股票代號」欄位"))?;
    let (pe, pb) = (column("本益比"), column("股價淨值比"));

    Ok(table
        .data
        .iter()
        .filter_map(|row| {
            let text = |index: Option<usize>| {
                index
                    .and_then(|i| row.get(i))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
            };
            let ratio = OfficialRatio {
                stock_symbol: text(Some(symbol))?.to_owned(),
                price_earning_ratio: text(pe).and_then(parse_ratio),
                price_to_book_ratio: text(pb).and_then(parse_ratio),
            };
            (!ratio.stock_symbol.is_empty() && ratio.has_any()).then_some(ratio)
        })
        .collect())
}

#[cfg(test)]
mod tests;
