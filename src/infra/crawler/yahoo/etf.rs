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
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    const ETF_0050: &str = include_str!("testdata/etf_holding_0050.html");
    const ETF_00632R: &str = include_str!("testdata/etf_holding_00632R.html");
    const ETF_00679B: &str = include_str!("testdata/etf_holding_00679B.html");

    fn day(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    #[test]
    fn parses_basic_info_and_nav_return() {
        let etf = parse("0050", ETF_0050).expect("0050 應解析成功");
        let p = &etf.profile;
        assert_eq!(p.full_name, "元大台灣卓越50證券投資信託基金");
        assert_eq!(p.issuer, "元大證券投資信託股份有限公司");
        assert_eq!(p.inception_date, day(2003, 6, 25));
        assert_eq!(p.benchmark, "臺灣50指數");
        assert_eq!(p.category, "指數型");
        assert!(!p.is_active);
        assert_eq!(p.region, "台灣");
        assert_eq!(p.dividend_frequency, "semi_annual");
        assert_eq!(p.management_fee, "0.3200%");
        assert_eq!(p.currency, "新台幣");
        assert_eq!(p.manager, "陳重銓");
        assert_eq!(p.website, "https://www.yuantaetfs.com");
        assert_eq!(p.total_assets, Some(dec!(2464976000000.00)));
        assert_eq!(p.total_assets_date, day(2026, 10, 21));
        assert_eq!(p.source, "yahoo");
        let nav = p.nav_return.as_ref().expect("有淨值報酬");
        assert_eq!(nav["1y"], json!(73.81));
        assert_eq!(nav["10y"], json!(20.56));
        assert_eq!(nav["date"], json!("2026-10-20"));
    }

    #[test]
    fn parses_holdings_industries_and_assets() {
        let etf = parse("0050", ETF_0050).expect("0050 應解析成功");
        let pf = &etf.portfolio;
        assert_eq!(pf.holdings.len(), 10);
        assert_eq!(pf.holdings_as_of, day(2026, 9, 1));
        assert_eq!(pf.holdings[0].rank, 1);
        assert_eq!(pf.holdings[0].holding_symbol, "2330");
        assert_eq!(pf.holdings[0].holding_name, "台積電");
        assert_eq!(pf.holdings[0].weight, dec!(56.45));
        assert_eq!(pf.industries_as_of, day(2026, 10, 17));
        assert_eq!(pf.industries[0].label, "半導體業");
        assert_eq!(pf.industries[0].weight, dec!(68.46));
        assert_eq!(pf.assets_as_of, day(2026, 10, 17));
        // 0.00 的「其他」不列。
        let labels: Vec<&str> = pf.assets.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["股票", "債券", "現金"]);
    }

    #[test]
    fn inverse_etf_has_no_top_holdings_and_bond_tickers_are_not_taiwan_symbols() {
        let inverse = parse("00632R", ETF_00632R).expect("00632R 應解析成功");
        assert!(inverse.portfolio.holdings.is_empty());
        assert_eq!(inverse.portfolio.holdings_as_of, None);
        assert_eq!(inverse.profile.dividend_frequency, "");
        assert_eq!(inverse.portfolio.assets[0].label, "債券");

        let bond = parse("00679B", ETF_00679B).expect("00679B 應解析成功");
        assert_eq!(bond.profile.dividend_frequency, "quarterly");
        assert_eq!(bond.profile.region, "美國");
        assert_eq!(bond.portfolio.holdings[0].holding_symbol, "");
        assert!(
            bond.portfolio.holdings[0]
                .holding_name
                .starts_with("US TREASURY")
        );
    }

    #[test]
    fn missing_or_invalid_store_returns_none() {
        assert!(parse("0050", "<html></html>").is_none());
        assert!(parse("0050", r#""QuoteETFStore":{"etfInfo":{}}"#).is_none());
        assert!(
            parse(
                "0050",
                r#""QuoteETFStore":{"etfInfo":{"data":{"isValidETF":false,"basicInfo":{}}}}"#
            )
            .is_none()
        );
        assert!(parse("0050", r#""QuoteETFStore":{broken"#).is_none());
    }

    #[test]
    fn minimal_store_parses_with_empty_sections() {
        let html = r#""QuoteETFStore":{"etfInfo":{"data":{"basicInfo":{"category":"主動型","features":[],"totalAssets":"0"},
            "performance":{"navReturn":{"nav_1y":"-"}},"portfolio":{"assetAllocation":null,
            "investIndustryWeights":{"detail":[{"name":"","weighting":"1"},{"name":"其他","weighting":"x"}]},
            "top10Holdings":{"holdingDetail":[{"name":"台積電","ticker":"2330.TWO","weighting":"9.9","date":"2026/09/01"}]}}}}}"#;
        let etf = parse("00999A", html).expect("最小資料應解析成功");
        assert!(etf.profile.is_active);
        assert_eq!(etf.profile.total_assets, None);
        assert_eq!(etf.profile.nav_return, None);
        assert!(etf.portfolio.industries.is_empty());
        assert!(etf.portfolio.assets.is_empty());
        assert_eq!(etf.portfolio.holdings[0].holding_symbol, "2330");
        assert_eq!(etf.portfolio.holdings_as_of, day(2026, 9, 1));
        assert_eq!(decimal(Some(&json!(1.5))), Some(dec!(1.5)));
        assert_eq!(decimal(Some(&json!(true))), None);
    }
}
