//! # 上櫃次日漲跌停價
//!
//! 來源：TPEx `www/zh-tw/afterTrading/otc?type=EW&response=json`
//! （上櫃股票每日收盤行情，不含權證、牛熊證，約 147 KB）。不帶 `date` 時回最新一個交易日，
//! 最後兩欄是「次日漲停價」「次日跌停價」，適用於資料日期的下一個交易日。
//!
//! 回應是 `{"tables":[{"date":"115/11/06","fields":[…],"data":[[…]]}]}`；欄位名稱尾端可能有空白。
//! 沒有漲跌幅限制的證券（債券 ETF、國外成分 ETF 等）次日漲停寫 `9,999.95`、次日跌停寫 `0.01`。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util::{self, datetime::roc_year_to_gregorian_year},
    infra::crawler::{share::ExchangePriceLimits, tpex},
};

/// 每日收盤行情的 JSON 回應。
#[derive(Debug, Default, Deserialize)]
struct Response {
    #[serde(default)]
    tables: Vec<Table>,
}

/// 回應中的一張表。
#[derive(Debug, Default, Deserialize)]
struct Table {
    #[serde(default)]
    date: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得最新一個交易日收盤行情附帶的上櫃次日漲跌停價。
///
/// # Errors
///
/// 請求失敗、沒有資料表、日期無法解析，或缺少代號／次日漲跌停欄位時回傳錯誤。
pub async fn visit() -> Result<ExchangePriceLimits> {
    let url = format!(
        "https://{host}/www/zh-tw/afterTrading/otc?type=EW&response=json",
        host = tpex::HOST
    );
    let response = util::http::get_json::<Response>(&url).await?;
    parse(response)
}

/// 解析民國日期 `115/11/06`。
fn parse_roc_date(text: &str) -> Option<NaiveDate> {
    let mut parts = text.trim().split('/');
    let year = roc_year_to_gregorian_year(parts.next()?.parse().ok()?);
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

/// 解析收盤行情回應。
fn parse(response: Response) -> Result<ExchangePriceLimits> {
    let table = response
        .tables
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("上櫃收盤行情沒有資料表"))?;
    let date = parse_roc_date(&table.date)
        .ok_or_else(|| anyhow!("上櫃收盤行情日期 {:?} 無法解析", table.date))?;
    let column = |name: &str| {
        table
            .fields
            .iter()
            .position(|field| field.trim() == name)
            .ok_or_else(|| anyhow!("上櫃收盤行情缺少「{name}」欄位"))
    };
    let (symbol, up, down) = (
        column("代號")?,
        column("次日漲停價")?,
        column("次日跌停價")?,
    );
    let limits = table
        .data
        .iter()
        .filter_map(|row| ExchangePriceLimits::row(row, symbol, down, up))
        .collect();
    Ok(ExchangePriceLimits { date, limits })
}

#[cfg(test)]
mod tests;
