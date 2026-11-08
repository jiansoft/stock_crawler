//! Yahoo ETF 頁（基本資料、持股、淨值報酬）的測試。

use rust_decimal_macros::dec;

use super::*;

const ETF_0050: &str = include_str!("../testdata/etf_holding_0050.html");
const ETF_00632R: &str = include_str!("../testdata/etf_holding_00632R.html");
const ETF_00679B: &str = include_str!("../testdata/etf_holding_00679B.html");

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
