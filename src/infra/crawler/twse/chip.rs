//! # 上市股票的三大法人買賣超與融資融券餘額
//!
//! | 函式 | 來源 |
//! |------|------|
//! | [`visit_institutional`] | TWSE `rwd/zh/fund/T86`（三大法人買賣超日報，單位：股） |
//! | [`visit_margin`] | TWSE `rwd/zh/marginTrading/MI_MARGN`（融資融券彙總，單位：張） |
//!
//! 兩者一次回傳全市場，休市日或尚未公布時沒有資料（回傳空集合，不是錯誤）。
//! 三大法人約 16:00 後公布、融資融券約 21:00 後公布。

use std::collections::HashMap;

use anyhow::Result;
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util,
    infra::crawler::{
        share::{
            InstitutionalFlow, MarginBalance,
            chip::{collect_rows, parse_count},
        },
        twse,
    },
};

/// T86 的回應：欄位以二維字串陣列回傳。
#[derive(Debug, Default, Deserialize)]
struct T86Response {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    data: Option<Vec<Vec<String>>>,
}

/// MI_MARGN 的回應：第一張表是信用交易統計，第二張才是個股彙總。
#[derive(Debug, Default, Deserialize)]
struct MarginResponse {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    tables: Vec<MarginTable>,
}

#[derive(Debug, Default, Deserialize)]
struct MarginTable {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    data: Option<Vec<Vec<String>>>,
}

/// 取得指定交易日上市股票的三大法人買賣超（`代號 → 買賣超`）。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤；休市或尚未公布回傳空集合。
pub async fn visit_institutional(date: NaiveDate) -> Result<HashMap<String, InstitutionalFlow>> {
    let url = format!(
        "https://www.{}/rwd/zh/fund/T86?date={}&selectType=ALLBUT0999&response=json",
        twse::HOST,
        date.format("%Y%m%d")
    );
    let response = util::http::get_json::<T86Response>(&url).await?;
    Ok(parse_institutional(response))
}

/// 取得指定交易日上市股票的融資融券餘額（`代號 → 餘額`）。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤；休市或尚未公布回傳空集合。
pub async fn visit_margin(date: NaiveDate) -> Result<HashMap<String, MarginBalance>> {
    let url = format!(
        "https://www.{}/rwd/zh/marginTrading/MI_MARGN?date={}&selectType=ALL&response=json",
        twse::HOST,
        date.format("%Y%m%d")
    );
    let response = util::http::get_json::<MarginResponse>(&url).await?;
    Ok(parse_margin(response))
}

/// T86 欄位：4 外陸資買賣超（不含外資自營商）、7 外資自營商買賣超、10 投信買賣超、11 自營商買賣超。
fn parse_institutional(response: T86Response) -> HashMap<String, InstitutionalFlow> {
    if response.stat != "OK" {
        return HashMap::new();
    }
    collect_rows(&response.data.unwrap_or_default(), |row| {
        Some(InstitutionalFlow {
            foreign: parse_count(row.get(4)?)? + parse_count(row.get(7)?)?,
            trust: parse_count(row.get(10)?)?,
            dealer: parse_count(row.get(11)?)?,
        })
    })
}

/// 個股彙總欄位：5 融資前日餘額、6 融資今日餘額、11 融券前日餘額、12 融券今日餘額。
fn parse_margin(response: MarginResponse) -> HashMap<String, MarginBalance> {
    if response.stat != "OK" {
        return HashMap::new();
    }
    let Some(rows) = response
        .tables
        .into_iter()
        .find(|table| {
            table
                .title
                .as_deref()
                .is_some_and(|title| title.contains("彙總"))
        })
        .and_then(|table| table.data)
    else {
        return HashMap::new();
    };
    collect_rows(&rows, |row| {
        Some(MarginBalance {
            margin_previous: parse_count(row.get(5)?)?,
            margin_today: parse_count(row.get(6)?)?,
            short_previous: parse_count(row.get(11)?)?,
            short_today: parse_count(row.get(12)?)?,
        })
    })
}

#[cfg(test)]
mod tests;
