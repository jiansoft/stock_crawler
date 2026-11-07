//! # 上市櫃公司每日重大訊息
//!
//! | 函式 | 來源 |
//! |------|------|
//! | [`visit_listed`] | 證交所 OpenAPI `opendata/t187ap04_L`（上市） |
//! | [`visit_otc`] | 櫃買 OpenAPI `mopsfin_t187ap04_O`（上櫃） |
//!
//! 兩者都只有「最近一天」的公告：證交所每天早上出表前一日的全部重大訊息，
//! 櫃買那份實測會再晚一天。欄位名稱兩邊不同（上市是中文、上櫃代號與名稱是英文），
//! 上市的「主旨」欄名尾端還多一個空白。
//!
//! 「發言時間」是去掉前導零的 `HHMMSS`（`70003` 是 07:00:03）。
//! 更名、面額變更這類公告在「公告期間」內每天重發，去重交給呼叫端。

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;

use crate::core::util::{self, datetime};

/// 上市每日重大訊息。
const LISTED_URL: &str = "https://openapi.twse.com.tw/v1/opendata/t187ap04_L";
/// 上櫃每日重大訊息。
const OTC_URL: &str = "https://www.tpex.org.tw/openapi/v1/mopsfin_t187ap04_O";

/// 一則重大訊息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialNews {
    /// 公司代號。
    pub stock_symbol: String,
    /// 公司名稱。
    pub company_name: String,
    /// 發言時間。
    pub spoken_at: NaiveDateTime,
    /// 主旨。
    pub subject: String,
    /// 符合條款（例如「第51款」）。
    pub clause: String,
    /// 說明全文。
    pub description: String,
}

/// 上市的原始資料列。
#[derive(Debug, Deserialize)]
struct ListedRaw {
    #[serde(rename = "公司代號")]
    code: String,
    #[serde(rename = "公司名稱")]
    name: String,
    #[serde(rename = "發言日期")]
    date: String,
    #[serde(rename = "發言時間")]
    time: String,
    #[serde(rename = "主旨 ", alias = "主旨")]
    subject: String,
    #[serde(rename = "符合條款", default)]
    clause: String,
    #[serde(rename = "說明", default)]
    description: String,
}

/// 上櫃的原始資料列。
#[derive(Debug, Deserialize)]
struct OtcRaw {
    #[serde(rename = "SecuritiesCompanyCode")]
    code: String,
    #[serde(rename = "CompanyName")]
    name: String,
    #[serde(rename = "發言日期")]
    date: String,
    #[serde(rename = "發言時間")]
    time: String,
    #[serde(rename = "主旨", alias = "主旨 ")]
    subject: String,
    #[serde(rename = "符合條款", default)]
    clause: String,
    #[serde(rename = "說明", default)]
    description: String,
}

/// 取得上市公司最近一天的重大訊息。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit_listed() -> Result<Vec<MaterialNews>> {
    let rows = util::http::get_json::<Vec<ListedRaw>>(LISTED_URL).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            build(
                row.code,
                row.name,
                &row.date,
                &row.time,
                row.subject,
                row.clause,
                row.description,
            )
        })
        .collect())
}

/// 取得上櫃公司最近一天的重大訊息。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit_otc() -> Result<Vec<MaterialNews>> {
    let rows = util::http::get_json::<Vec<OtcRaw>>(OTC_URL).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            build(
                row.code,
                row.name,
                &row.date,
                &row.time,
                row.subject,
                row.clause,
                row.description,
            )
        })
        .collect())
}

/// 整理成 [`MaterialNews`]；日期或時間無法解析的列丟棄（無從判斷新舊）。
fn build(
    code: String,
    name: String,
    date: &str,
    time: &str,
    subject: String,
    clause: String,
    description: String,
) -> Option<MaterialNews> {
    let stock_symbol = code.trim().to_string();
    if stock_symbol.is_empty() {
        return None;
    }
    Some(MaterialNews {
        stock_symbol,
        company_name: name.trim().to_string(),
        spoken_at: NaiveDateTime::new(parse_date(date)?, parse_time(time)?),
        subject: normalize(&subject),
        clause: clause.trim().to_string(),
        description: description.replace("\r\n", "\n").trim().to_string(),
    })
}

/// 民國日期 `1151005` → 2026-10-05。
fn parse_date(raw: &str) -> Option<NaiveDate> {
    datetime::parse_taiwan_date_short(raw.trim())
}

/// `70003` → 07:00:03（左補零成六碼）。
fn parse_time(raw: &str) -> Option<NaiveTime> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 6 || !raw.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    NaiveTime::parse_from_str(&format!("{raw:0>6}"), "%H%M%S").ok()
}

/// 主旨的換行是排版用的，接成一行。
fn normalize(subject: &str) -> String {
    subject.lines().map(str::trim).collect::<Vec<_>>().join("")
}

#[cfg(test)]
mod tests;
