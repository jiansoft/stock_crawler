//! # Yahoo 財務報表採集器
//!
//! 採集 Yahoo 股市「財務」分頁的三大報表：
//!
//! | 子模組 | 頁面 | Yahoo 服務 | 支援期別 |
//! |--------|------|------------|----------|
//! | [`income_statement`] | `/quote/{代號}/income-statement` | `StockServices.incomeStatements-growthAnalyses` | 單季、累計、年度 |
//! | [`balance_sheet`] | `/quote/{代號}/balance-sheet` | `StockServices.balanceSheets` | 單季 |
//! | [`cash_flow`] | `/quote/{代號}/cash-flow-statement` | `StockServices.cashFlowStatements` | 單季、累計、年度 |
//!
//! ## 為什麼打 JSON API 而不是解析頁面
//!
//! 頁面雖然是伺服器端渲染，但內嵌資料只有預設分頁（最近 20 季）；年度與累計分頁是前端
//! 另外呼叫 `_td-stock/api/resource/StockServices.*` 取得的。直接打同一支 API 可以一次拿到
//! 全部期別與完整歷史（最多約自 2015 年起），也不必依賴畫面結構。
//!
//! ## 回應的共同規則（2026-09 實測）
//!
//! - 金額單位為**元**，每股數值單位為元；數值一律是字串（如 `"-52332000.00"`、`"150932000"`），
//!   少數欄位可能為 `null`。解析成 `Option<Decimal>`，`null` 與空字串視為 `None`，**不可**當成 0。
//! - `date` 是期別的**起始月份**（`2026-06-01T00:00:00+08:00`）：單季與累計為該季最後一個月，
//!   年度為該年 1 月。本模組換算成 [`FiscalPeriod`]（年度＋季別）。
//! - 代號可帶或不帶 `.TW`／`.TWO` 後綴；不存在的代號回 HTTP 404
//!   （轉成 [`YahooPageNotFoundError`]），ETF 等沒有財報的標的回空陣列。
//! - 資產負債表的 `period=year` 會混入逐日垃圾資料（2020-05～06 每天一筆），
//!   而且頁面本身也只提供單季，因此資產負債表只採單季。
//! - 金融業許多欄位（流動資產、銷售費用等）回 `0.00`，是 Yahoo 的原始值，照實保留。

/// 資產負債表。
pub mod balance_sheet;
/// 現金流量表。
pub mod cash_flow;
/// 損益表。
pub mod income_statement;

use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, de::DeserializeOwned};

use crate::{
    core::util::{self, http},
    infra::crawler::yahoo::{HOST, dividend::YahooPageNotFoundError},
};

/// 一次請求的筆數上限；大於實際可取得的歷史筆數即可一次取完。
const MAX_LIMIT: u32 = 100;

/// 報表期別（對應 Yahoo 的 `period` 參數）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportPeriod {
    /// 單季（`quarter`）。
    Quarter,
    /// 年初累計至該季（`quarterSum`）。
    CumulativeQuarter,
    /// 年度（`year`）。
    Year,
}

impl ReportPeriod {
    /// Yahoo API 使用的 `period` 參數值。
    fn as_param(self) -> &'static str {
        match self {
            Self::Quarter => "quarter",
            Self::CumulativeQuarter => "quarterSum",
            Self::Year => "year",
        }
    }
}

/// 財報所屬期間。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FiscalPeriod {
    /// 西元年度。
    pub year: i32,
    /// 季別 1～4；年度報表為 `None`。
    pub quarter: Option<u8>,
}

/// 單一期別的財務報表，`T` 為各報表的項目。
#[derive(Debug, Clone, PartialEq)]
pub struct FinancialStatement<T> {
    /// 股票代號（不含 `.TW`／`.TWO` 後綴）。
    pub stock_symbol: String,
    /// 報表期別。
    pub report_period: ReportPeriod,
    /// 所屬期間。
    pub fiscal_period: FiscalPeriod,
    /// 報表項目。
    pub items: T,
}

/// Yahoo 回應中的單筆原始資料：共同的 `symbol`、`date` 加上各報表自己的項目。
#[derive(Debug, Deserialize)]
struct RawStatement<T> {
    symbol: String,
    date: String,
    #[serde(flatten)]
    items: T,
}

/// 組出 Yahoo 財報 API 的網址。
fn build_url(service: &str, stock_symbol: &str, period: ReportPeriod) -> String {
    format!(
        "https://{HOST}/_td-stock/api/resource/StockServices.{service};limit={MAX_LIMIT};period={period};sortBy=-date;symbol={stock_symbol}",
        period = period.as_param()
    )
}

/// 呼叫 Yahoo 財報 API 並反序列化回應。
///
/// 404 轉成 [`YahooPageNotFoundError`]，讓呼叫端能用
/// [`crate::infra::crawler::yahoo::dividend::is_page_not_found_error`] 辨識「代號不存在」。
async fn fetch<RES: DeserializeOwned>(
    service: &str,
    stock_symbol: &str,
    period: ReportPeriod,
) -> Result<RES> {
    let url = build_url(service, stock_symbol, period);
    let response = http::get_response(&url, None).await?;
    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(YahooPageNotFoundError {
            stock_symbol: stock_symbol.to_string(),
            url,
        }
        .into());
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("Error reading Yahoo {service} body from {url}"))?;
    if !status.is_success() {
        return Err(anyhow!(
            "Yahoo {service} returned HTTP {status} for {url}. Body: {}",
            util::text::truncate(&body, 200)
        ));
    }

    serde_json::from_str(&body).with_context(|| {
        format!(
            "Error parsing Yahoo {service} JSON from {url}. Body: {}",
            util::text::truncate(&body, 200)
        )
    })
}

/// 把原始資料列轉成 [`FinancialStatement`]。
///
/// 採嚴格模式：任何一筆的日期無法對應到期別就整批失敗，
/// 寧可這次不採集，也不要把錯置期別的數字交給下游。
fn parse_statements<T>(
    rows: Vec<RawStatement<T>>,
    period: ReportPeriod,
) -> Result<Vec<FinancialStatement<T>>> {
    rows.into_iter()
        .map(|row| {
            let fiscal_period = parse_fiscal_period(&row.date, period).with_context(|| {
                format!(
                    "Unexpected Yahoo financial statement date {} for {} ({})",
                    row.date,
                    row.symbol,
                    period.as_param()
                )
            })?;

            Ok(FinancialStatement {
                stock_symbol: strip_exchange_suffix(&row.symbol).to_string(),
                report_period: period,
                fiscal_period,
                items: row.items,
            })
        })
        .collect()
}

/// 由 Yahoo 的 `date`（期別起始月份）換算所屬期間。
///
/// 單季與累計的月份必須是 3／6／9／12，年度必須是 1 月，其餘一律視為異常。
fn parse_fiscal_period(date: &str, period: ReportPeriod) -> Result<FiscalPeriod> {
    let date = DateTime::parse_from_rfc3339(date)
        .with_context(|| format!("Invalid date: {date}"))?
        .date_naive();
    let month = date.month();

    let quarter = match period {
        ReportPeriod::Year if month == 1 => None,
        ReportPeriod::Quarter | ReportPeriod::CumulativeQuarter if month % 3 == 0 => {
            Some((month / 3) as u8)
        }
        _ => return Err(anyhow!("month {month} does not match period")),
    };

    Ok(FiscalPeriod {
        year: date.year(),
        quarter,
    })
}

/// 去掉 Yahoo 代號的交易所後綴（`8042.TWO` → `8042`）。
fn strip_exchange_suffix(symbol: &str) -> &str {
    symbol.split_once('.').map_or(symbol, |(code, _)| code)
}

/// 反序列化 Yahoo 的數值欄位：字串或數字轉成 `Decimal`，`null`、空字串與 `-` 為 `None`。
fn deserialize_decimal<'de, D>(deserializer: D) -> std::result::Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawNumber {
        Text(String),
        Number(serde_json::Number),
    }

    let text = match Option::<RawNumber>::deserialize(deserializer)? {
        None => return Ok(None),
        Some(RawNumber::Text(text)) => text,
        Some(RawNumber::Number(number)) => number.to_string(),
    };
    let text = text.trim();
    if text.is_empty() || text == "-" {
        return Ok(None);
    }

    Decimal::from_str(text)
        .or_else(|_| Decimal::from_scientific(text))
        .map(Some)
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn test_parse_fiscal_period() {
        let quarter = parse_fiscal_period("2026-06-01T00:00:00+08:00", ReportPeriod::Quarter)
            .expect("quarter date");
        assert_eq!(
            quarter,
            FiscalPeriod {
                year: 2026,
                quarter: Some(2)
            }
        );

        let cumulative =
            parse_fiscal_period("2025-12-01T00:00:00+08:00", ReportPeriod::CumulativeQuarter)
                .expect("cumulative date");
        assert_eq!(cumulative.quarter, Some(4));

        let year = parse_fiscal_period("2025-01-01T00:00:00+08:00", ReportPeriod::Year)
            .expect("year date");
        assert_eq!(
            year,
            FiscalPeriod {
                year: 2025,
                quarter: None
            }
        );
    }

    /// 資產負債表 `period=year` 混入的逐日資料（2020-06-10）必須被擋下。
    #[test]
    fn test_parse_fiscal_period_rejects_mismatched_month() {
        assert!(parse_fiscal_period("2020-06-10T00:00:00+08:00", ReportPeriod::Year).is_err());
        assert!(parse_fiscal_period("2026-05-01T00:00:00+08:00", ReportPeriod::Quarter).is_err());
        assert!(parse_fiscal_period("not-a-date", ReportPeriod::Quarter).is_err());
    }

    #[test]
    fn test_strip_exchange_suffix() {
        assert_eq!(strip_exchange_suffix("8042.TWO"), "8042");
        assert_eq!(strip_exchange_suffix("2330.TW"), "2330");
        assert_eq!(strip_exchange_suffix("2330"), "2330");
    }

    #[test]
    fn test_deserialize_decimal() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "deserialize_decimal")]
            value: Option<Decimal>,
        }

        let parse = |json: &str| serde_json::from_str::<Row>(json).expect("row").value;

        assert_eq!(
            parse(r#"{"value":"-52332000.00"}"#),
            Some(dec!(-52332000.00))
        );
        assert_eq!(parse(r#"{"value":"150932000"}"#), Some(dec!(150932000)));
        assert_eq!(parse(r#"{"value":1.5}"#), Some(dec!(1.5)));
        assert_eq!(parse(r#"{"value":null}"#), None);
        assert_eq!(parse(r#"{"value":""}"#), None);
        assert_eq!(parse(r#"{}"#), None);
        assert!(serde_json::from_str::<Row>(r#"{"value":"abc"}"#).is_err());
    }

    #[test]
    fn test_build_url() {
        assert_eq!(
            build_url(
                "cashFlowStatements",
                "8042.TWO",
                ReportPeriod::CumulativeQuarter
            ),
            "https://tw.stock.yahoo.com/_td-stock/api/resource/StockServices.cashFlowStatements;limit=100;period=quarterSum;sortBy=-date;symbol=8042.TWO"
        );
    }
}
