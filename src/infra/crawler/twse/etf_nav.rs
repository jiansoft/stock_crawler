//! # 證交所 MIS「ETF 預估淨值」
//!
//! `https://mis.twse.com.tw/stock/data/all_etf.txt` 一次回傳全部上市櫃 ETF（依投信分組，
//! `a1[].msgArray[]`），每筆欄位：
//!
//! | 鍵 | 意義 |
//! |----|------|
//! | `a` | 代號 |
//! | `c` | 已發行受益權單位數 |
//! | `d` | 與前日已發行單位差異 |
//! | `e` | 成交價 |
//! | `f` | 投信預估淨值 |
//! | `g` | 預估折溢價幅度（%） |
//! | `h` | 前一營業日單位淨值 |
//! | `i`／`j` | 資料日期（YYYYMMDD）／時間 |
//!
//! 同一份檔案裡數值有時是 JSON number、有時是字串，字串還可能帶千分位逗號；
//! `-` 與空字串代表沒有資料。投信約在 17:00 前更新完當天的預估淨值。

use std::str::FromStr;

use anyhow::Result;
use chrono::{FixedOffset, NaiveDate, NaiveTime, TimeZone};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde_json::Value;

use crate::{core::util, domain::etf::EtfNavRecord};

/// 全部 ETF 預估淨值。
const URL: &str = "https://mis.twse.com.tw/stock/data/all_etf.txt";

/// 抓取全部 ETF 的預估淨值、折溢價與受益單位。
pub async fn visit() -> Result<Vec<EtfNavRecord>> {
    let body = util::http::get_json::<Value>(URL).await?;
    Ok(parse(&body))
}

/// 把 MIS 的原始 JSON 整理成 [`EtfNavRecord`]；缺代號、日期或單位數的列略過。
pub(crate) fn parse(body: &Value) -> Vec<EtfNavRecord> {
    let Some(groups) = body.get("a1").and_then(Value::as_array) else {
        return Vec::new();
    };
    groups
        .iter()
        .filter_map(|group| group.get("msgArray").and_then(Value::as_array))
        .flatten()
        .filter_map(parse_row)
        .collect()
}

fn parse_row(row: &Value) -> Option<EtfNavRecord> {
    let symbol = row.get("a")?.as_str()?.trim();
    if symbol.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(row.get("i")?.as_str()?.trim(), "%Y%m%d").ok()?;
    let outstanding_units = number(row, "c")?.trunc().to_i64()?;
    let units_change = number(row, "d")
        .and_then(|value| value.trunc().to_i64())
        .unwrap_or(0);
    let source_time = row
        .get("j")
        .and_then(Value::as_str)
        .and_then(|time| NaiveTime::parse_from_str(time.trim(), "%H:%M:%S").ok())
        .and_then(|time| taipei().from_local_datetime(&date.and_time(time)).single());
    Some(EtfNavRecord {
        date,
        stock_symbol: symbol.to_string(),
        outstanding_units,
        units_change,
        market_price: positive(number(row, "e")),
        estimated_nav: positive(number(row, "f")),
        premium_rate: number(row, "g"),
        previous_nav: positive(number(row, "h")),
        source_time,
    })
}

fn taipei() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 合法")
}

/// 數值欄位：number 或字串（可帶千分位），`-`、空字串或無法解析為 `None`。
fn number(row: &Value, key: &str) -> Option<Decimal> {
    match row.get(key)? {
        Value::Number(number) => Decimal::from_str(&number.to_string())
            .ok()
            .or_else(|| number.as_f64().and_then(Decimal::from_f64_retain)),
        Value::String(text) => {
            let cleaned: String = text.trim().chars().filter(|c| *c != ',').collect();
            Decimal::from_str(&cleaned).ok()
        }
        _ => None,
    }
}

/// 價格與淨值只接受正值；0 代表來源沒有資料。
fn positive(value: Option<Decimal>) -> Option<Decimal> {
    value.filter(|v| *v > Decimal::ZERO)
}

#[cfg(test)]
mod tests;
