//! # Yahoo ETF 頁（基本資料、持股、淨值報酬）
//!
//! `https://tw.stock.yahoo.com/quote/{代號}/holding` 的 HTML 內嵌了 `"QuoteETFStore":{...}` JSON，
//! 一頁就含：
//!
//! - `etfInfo.data.basicInfo`：全名、發行投信、成立日、追蹤指數、類型、投資地區、費率、配息頻率、資產規模
//! - `etfInfo.data.performance.navReturn`：淨值報酬（1 年以上為年化）
//! - `etfInfo.data.portfolio`：資產分佈、行業比重、前十大持股（各自帶資料日期）
//!
//! 代號不必帶 `.TW`／`.TWO`，Yahoo 會自己對應市場。反向、期貨型 ETF 沒有前十大持股（`holdingDetail` 為 null）。

use std::str::FromStr;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};

use crate::{
    core::util,
    domain::etf::{
        EtfAllocationRecord, EtfHoldingRecord, EtfPortfolio, EtfProfileRecord, classify,
    },
    infra::crawler::yahoo::HOST,
};

/// 一檔 ETF 在 Yahoo 的基本資料與持股。
#[derive(Debug, Clone, PartialEq)]
pub struct YahooEtf {
    /// 基本資料（分類欄位 `strategy`、`themes` 留空，由呼叫端推斷）。
    pub profile: EtfProfileRecord,
    /// 持股、行業比重與資產分佈。
    pub portfolio: EtfPortfolio,
}

/// 抓一檔 ETF；頁面不存在（404）或頁內沒有 ETF 資料時回 `None`。
pub async fn visit(stock_symbol: &str) -> Result<Option<YahooEtf>> {
    let url = format!("https://{HOST}/quote/{stock_symbol}/holding");
    let response = util::http::get_response(&url, None).await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        anyhow::bail!("Yahoo ETF 頁回應 {}：{stock_symbol}", response.status());
    }
    let html = response
        .text()
        .await
        .context("Error parsing Yahoo ETF page text")?;
    Ok(parse(stock_symbol, &html))
}

/// 從頁面 HTML 取出 `QuoteETFStore` 並整理；找不到或不是有效 ETF 時回 `None`。
pub(crate) fn parse(stock_symbol: &str, html: &str) -> Option<YahooEtf> {
    const KEY: &str = "\"QuoteETFStore\":";
    let start = html.find(KEY)? + KEY.len();
    let store: Value = serde_json::Deserializer::from_str(&html[start..])
        .into_iter::<Value>()
        .next()?
        .ok()?;
    let data = store.pointer("/etfInfo/data")?;
    if data.get("isValidETF").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let basic = data.get("basicInfo").filter(|v| v.is_object())?;
    Some(YahooEtf {
        profile: profile(stock_symbol, basic, data.pointer("/performance")),
        portfolio: portfolio(stock_symbol, data.get("portfolio")),
    })
}

fn profile(stock_symbol: &str, basic: &Value, performance: Option<&Value>) -> EtfProfileRecord {
    let features_active = basic
        .get("features")
        .and_then(Value::as_array)
        .is_some_and(|features| features.iter().any(|f| f.as_str() == Some("ACTIVE")));
    let category = text(basic, "/category");
    EtfProfileRecord {
        stock_symbol: stock_symbol.to_string(),
        full_name: text(basic, "/fundDisplayName/local"),
        issuer: text(basic, "/providerCompanyName/local"),
        inception_date: date(basic.pointer("/inceptionDate")),
        benchmark: text(basic, "/primaryProspectusBenchmark"),
        is_active: features_active || category.contains("主動"),
        category,
        region: text(basic, "/investRegion/local"),
        dividend_frequency: classify::dividend_frequency(&text(basic, "/divCashFreq")).to_string(),
        management_fee: text(basic, "/managementfee"),
        custodian_fee: text(basic, "/custodianfee"),
        currency: text(basic, "/currency/local"),
        exchange_name: text(basic, "/exchangeName/local"),
        manager: text(basic, "/fundManagerName/local"),
        website: text(basic, "/providerCompany/website"),
        total_assets: decimal(basic.pointer("/totalAssets")).filter(|v| *v > Decimal::ZERO),
        total_assets_date: date(basic.pointer("/totalAssetsDate")),
        nav_return: performance.and_then(nav_return),
        source: "yahoo".to_string(),
        ..Default::default()
    }
}

/// 淨值報酬整理成 `{"date": "2026-10-20", "1w": -4.42, ..., "10y": 20.56}`；全部缺值時為 `None`。
fn nav_return(performance: &Value) -> Option<Value> {
    let source = performance.get("navReturn")?;
    let keys = [
        ("1w", "nav_1w"),
        ("1m", "nav_1m"),
        ("3m", "nav_3m"),
        ("6m", "nav_6m"),
        ("ytd", "navYtd"),
        ("1y", "nav_1y"),
        ("2y", "nav_2yl"),
        ("3y", "nav_3yl"),
        ("5y", "nav_5yl"),
        ("10y", "nav_10yl"),
    ];
    let mut out = Map::new();
    for (key, source_key) in keys {
        if let Some(value) = source
            .get(source_key)
            .and_then(Value::as_str)
            .and_then(|s| s.trim().parse::<f64>().ok())
        {
            out.insert(key.to_string(), json!(value));
        }
    }
    if out.is_empty() {
        return None;
    }
    if let Some(day) = date(performance.get("returnDate")) {
        out.insert("date".to_string(), json!(day.to_string()));
    }
    Some(Value::Object(out))
}

fn portfolio(stock_symbol: &str, portfolio: Option<&Value>) -> EtfPortfolio {
    let mut out = EtfPortfolio {
        stock_symbol: stock_symbol.to_string(),
        ..Default::default()
    };
    let Some(portfolio) = portfolio else {
        return out;
    };

    if let Some(top10) = portfolio.get("top10Holdings") {
        let rows = top10
            .get("holdingDetail")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        out.holdings = rows
            .iter()
            .filter_map(|row| {
                let name = row.get("name")?.as_str()?.trim();
                let weight = decimal(row.get("weighting"))?;
                (!name.is_empty()).then(|| (name.to_string(), weight, taiwan_ticker(row)))
            })
            .zip(1i16..)
            .map(
                |((holding_name, weight, holding_symbol), rank)| EtfHoldingRecord {
                    rank,
                    holding_symbol,
                    holding_name,
                    weight,
                },
            )
            .collect();
        if !out.holdings.is_empty() {
            out.holdings_as_of =
                date(top10.get("date")).or_else(|| date(rows.first().and_then(|r| r.get("date"))));
        }
    }

    if let Some(industries) = portfolio.get("investIndustryWeights") {
        out.industries = industries
            .get("detail")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| allocation(row.get("name"), row.get("weighting")))
            .collect();
        if !out.industries.is_empty() {
            out.industries_as_of = date(industries.get("date"));
        }
    }

    if let Some(assets) = portfolio.get("assetAllocation").filter(|v| v.is_object()) {
        out.assets = [
            ("股票", "stock"),
            ("債券", "bond"),
            ("現金", "cash"),
            ("其他", "other"),
        ]
        .iter()
        .filter_map(|(label, key)| {
            let weight = decimal(assets.get(*key))?;
            (weight > Decimal::ZERO).then(|| EtfAllocationRecord {
                label: (*label).to_string(),
                weight,
            })
        })
        .collect();
        if !out.assets.is_empty() {
            out.assets_as_of = date(assets.get("date"));
        }
    }
    out
}

fn allocation(name: Option<&Value>, weight: Option<&Value>) -> Option<EtfAllocationRecord> {
    let label = name?.as_str()?.trim();
    (!label.is_empty()).then_some(())?;
    Some(EtfAllocationRecord {
        label: label.to_string(),
        weight: decimal(weight)?,
    })
}

/// 台股成分股的代號（`2330.TW` → `2330`）；債券的 ISIN 等非台股代號為空字串。
fn taiwan_ticker(row: &Value) -> String {
    row.get("ticker")
        .and_then(Value::as_str)
        .and_then(|ticker| {
            ticker
                .strip_suffix(".TWO")
                .or_else(|| ticker.strip_suffix(".TW"))
        })
        .unwrap_or_default()
        .to_string()
}

fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn decimal(value: Option<&Value>) -> Option<Decimal> {
    match value? {
        Value::String(s) => Decimal::from_str(s.trim().trim_end_matches('%')).ok(),
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
        _ => None,
    }
}

/// Yahoo 的日期是 `2026/10/17`。
fn date(value: Option<&Value>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value?.as_str()?.trim(), "%Y/%m/%d").ok()
}

#[cfg(test)]
mod tests;
