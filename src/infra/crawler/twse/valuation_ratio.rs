//! # 上市個股日本益比與股價淨值比
//!
//! 來源：TWSE `exchangeReport/BWIBBU_d?response=json&date=YYYYMMDD&selectType=ALL`
//! （個股日本益比、殖利率及股價淨值比），可查任意歷史日期，2012 年起實測可用。
//!
//! **欄位順序隨年份不同**：2012 年是「證券代號、證券名稱、本益比、殖利率、股價淨值比」，
//! 2020 年起變成「證券代號、證券名稱、收盤價、殖利率、股利年度、本益比、股價淨值比、
//! 財報年/季」，所以一律依 `fields` 的欄名找位置。虧損股的本益比是 `-`。
//! 非交易日 `stat` 不是 `OK`，視為沒有資料。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util,
    domain::quote::official_ratio::{OfficialRatio, parse_ratio},
    infra::crawler::twse,
};

/// BWIBBU_d 的回應主體；資料列混有字串與數字（股利年度），以 JSON 值接收。
#[derive(Debug, Default, Deserialize)]
struct BwibbuResponse {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得指定日期的上市個股本益比與股價淨值比；非交易日回傳空陣列。
///
/// # Errors
///
/// 請求失敗、回應不是預期的 JSON，或缺少「證券代號」欄位時回傳錯誤。
pub async fn visit(date: NaiveDate) -> Result<Vec<OfficialRatio>> {
    let url = format!(
        "https://www.{host}/exchangeReport/BWIBBU_d?response=json&date={date}&selectType=ALL",
        host = twse::HOST,
        date = date.format("%Y%m%d")
    );
    let response = util::http::get_json::<BwibbuResponse>(&url).await?;
    parse(response)
}

/// 解析 BWIBBU_d 回應。
fn parse(response: BwibbuResponse) -> Result<Vec<OfficialRatio>> {
    if response.stat != "OK" {
        return Ok(Vec::new());
    }
    let column = |name: &str| response.fields.iter().position(|field| field == name);
    let symbol = column("證券代號").ok_or_else(|| anyhow!("BWIBBU_d 缺少「證券代號」欄位"))?;
    let (pe, pb) = (column("本益比"), column("股價淨值比"));

    Ok(response
        .data
        .iter()
        .filter_map(|row| {
            let text = |index: Option<usize>| index.and_then(|i| row.get(i)).map(cell_text);
            let ratio = OfficialRatio {
                stock_symbol: text(Some(symbol))?.trim().to_owned(),
                price_earning_ratio: text(pe).as_deref().and_then(parse_ratio),
                price_to_book_ratio: text(pb).as_deref().and_then(parse_ratio),
            };
            (!ratio.stock_symbol.is_empty() && ratio.has_any()).then_some(ratio)
        })
        .collect())
}

/// JSON 儲存格轉字串（數字欄位如股利年度也可能出現）。
fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
