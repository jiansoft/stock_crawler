use anyhow::{Result, anyhow};
use regex::Regex;
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::{core::util::http, core::util::http::element, infra::crawler::wespai::HOST};

#[derive(Debug, Clone, Deserialize, Serialize)]
/// Wespai 財務指標頁面的單筆獲利資料。
pub struct Profit {
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: String,
    /// 股票代號。
    pub security_code: String,
    /// 營業毛利率
    pub gross_profit: Decimal,
    /// 營業利益率
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率
    pub pre_tax_income: Decimal,
    /// 稅後淨利率
    pub net_income: Decimal,
    /// 每股淨值
    pub net_asset_value_per_share: Decimal,
    /// 每股營收
    pub sales_per_share: Decimal,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率
    pub return_on_equity: Decimal,
    /// 資產報酬率
    pub return_on_assets: Decimal,
    /// 年度
    pub year: i32,
}

impl Profit {
    /// 建立一筆指定年度與股票代號的 `Profit` 預設值。
    pub fn new(year: i32, security_code: String) -> Self {
        Profit {
            quarter: "".to_string(),
            security_code,
            gross_profit: Default::default(),
            operating_profit_margin: Default::default(),
            pre_tax_income: Default::default(),
            net_income: Default::default(),
            net_asset_value_per_share: Default::default(),
            sales_per_share: Default::default(),
            earnings_per_share: Default::default(),
            profit_before_tax: Default::default(),
            return_on_equity: Default::default(),
            return_on_assets: Default::default(),
            year,
        }
    }
}

/// 抓取年報
///
/// 只負責 HTTP 抓取，解析交給 [`parse_profit_html`]
/// （fetch/parse 分離，解析邏輯才能被 fixture 單元測試覆蓋）。
pub async fn visit() -> Result<Vec<Profit>> {
    let url = format!("https://stock.{}/profit", HOST);
    let ua = http::user_agent::gen_random_ua();
    let mut headers = HeaderMap::new();

    headers.insert("Referer", url.parse()?);
    headers.insert("User-Agent", ua.parse()?);
    headers.insert("content-length", "0".parse()?);

    let text = http::get(&url, Some(headers)).await?;
    parse_profit_html(&text)
}

/// 解析 Wespai 財務指標頁（`stock.wespai.com/profit`）的 HTML。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O，
/// 可用 `testdata/profit.html` fixture 直接驗證。
///
/// # 解析流程
/// 1. 年度取自頁面標題 `body > h1 > a` 文字中的四位數年份；
///    取不到（或為 0）視為頁面異常，直接回錯——年度是每一筆資料的 key，
///    寧可整頁失敗也不要把整批資料掛在錯誤年度下。
/// 2. 資料列位於 `#example > tbody > tr`；各欄以固定的 nth-child 對應：
///    第 1 欄＝代號、4~9 欄＝毛利率/營益率/稅前/稅後/每股淨值/每股營收、
///    11~14 欄＝每股稅前淨利/ROE/ROA/EPS。無法解析的數值欄以 0 落地。
fn parse_profit_html(text: &str) -> Result<Vec<Profit>> {
    let document = Html::parse_document(text);
    let selector = match Selector::parse("body > h1 > a") {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };
    let year = match document.select(&selector).next() {
        None => {
            return Err(anyhow!("Failed to select .next()"));
        }
        Some(year) => year,
    };
    let year = match year.text().next() {
        None => {
            return Err(anyhow!("Failed to parse year raw({:?})", year));
        }
        Some(year) => year,
    };
    let re = Regex::new(r"\d{4}")?;
    let mut profit_year = 0;
    if let Some(caps) = re.captures(year)
        && let Some(q) = caps.get(0)
    {
        profit_year = q.as_str().parse::<i32>()?
    }

    if profit_year == 0 {
        return Err(anyhow!("profit_year is zero"));
    }

    let selector = match Selector::parse("#example > tbody > tr") {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };
    let mut profits = Vec::with_capacity(2048);

    for element in document.select(&selector) {
        //let tds: Vec<&str> = element.text().collect();
        //println!("tds:{:#?}",tds);
        let security_code = match element::parse_value(&element, "td:nth-child(1)") {
            None => continue,
            Some(security_code) => security_code,
        };

        let mut p = Profit::new(profit_year, security_code);
        //grossProfit := s.Find(fmt.Sprintf("td:nth-child(%d)", 3+jumpColumnCount)).Text()
        p.gross_profit = element::parse_to_decimal(&element, "td:nth-child(4)");
        //	operatingProfitMargin := s.Find(fmt.Sprintf("td:nth-child(%d)", 4+jumpColumnCount)).Text()
        p.operating_profit_margin = element::parse_to_decimal(&element, "td:nth-child(5)");
        //preTaxIncome := s.Find(fmt.Sprintf("td:nth-child(%d)", 5+jumpColumnCount)).Text()
        p.pre_tax_income = element::parse_to_decimal(&element, "td:nth-child(6)");
        //netIncome := s.Find(fmt.Sprintf("td:nth-child(%d)", 6+jumpColumnCount)).Text()
        p.net_income = element::parse_to_decimal(&element, "td:nth-child(7)");
        //netAssetValuePerShare := s.Find(fmt.Sprintf("td:nth-child(%d)", 7+jumpColumnCount)).Text()
        p.net_asset_value_per_share = element::parse_to_decimal(&element, "td:nth-child(8)");
        //salesPerShare := s.Find(fmt.Sprintf("td:nth-child(%d)", 8+jumpColumnCount)).Text()
        p.sales_per_share = element::parse_to_decimal(&element, "td:nth-child(9)");
        //earningsPerShare := s.Find(fmt.Sprintf("td:nth-child(%d)", 13+jumpColumnCount)).Text()
        p.earnings_per_share = element::parse_to_decimal(&element, "td:nth-child(14)");
        //profitBeforeTax := s.Find(fmt.Sprintf("td:nth-child(%d)", 10+jumpColumnCount)).Text()
        p.profit_before_tax = element::parse_to_decimal(&element, "td:nth-child(11)");
        //returnOnEquity := s.Find(fmt.Sprintf("td:nth-child(%d)", 11+jumpColumnCount)).Text()
        p.return_on_equity = element::parse_to_decimal(&element, "td:nth-child(12)");
        //returnOnAssets := s.Find(fmt.Sprintf("td:nth-child(%d)", 12+jumpColumnCount)).Text()
        p.return_on_assets = element::parse_to_decimal(&element, "td:nth-child(13)");
        //tracing::debug!("p:{:#?}", p);
        profits.push(p);
    }

    Ok(profits)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use rust_decimal_macros::dec;

    /// 以貼近真實頁面形狀的 fixture 驗證整頁解析流程。
    #[test]
    fn parse_profit_html_parses_fixture_rows() {
        // include_str! 的路徑相對於本檔案（wespai/profit.rs）→ wespai/testdata/。
        const FIXTURE: &str = include_str!("testdata/profit.html");

        let profits = parse_profit_html(FIXTURE).unwrap();

        assert_eq!(profits.len(), 2);

        // 年度來自標題「2024年報獲利能力」中的四位數。
        let tsmc = &profits[0];
        assert_eq!(tsmc.year, 2024);
        assert_eq!(tsmc.security_code, "2330");
        assert_eq!(tsmc.gross_profit, dec!(56.12));
        assert_eq!(tsmc.operating_profit_margin, dec!(45.68));
        assert_eq!(tsmc.pre_tax_income, dec!(48.63));
        assert_eq!(tsmc.net_income, dec!(43.06));
        assert_eq!(tsmc.net_asset_value_per_share, dec!(155.86));
        assert_eq!(tsmc.sales_per_share, dec!(104.88));
        assert_eq!(tsmc.profit_before_tax, dec!(51.01));
        assert_eq!(tsmc.return_on_equity, dec!(30.24));
        assert_eq!(tsmc.return_on_assets, dec!(19.35));
        assert_eq!(tsmc.earnings_per_share, dec!(45.25));

        // 金融股的毛利率欄是「-」→ 以 0 落地，不影響其他欄位。
        let esun = &profits[1];
        assert_eq!(esun.security_code, "2884");
        assert_eq!(esun.gross_profit, Decimal::ZERO);
        assert_eq!(esun.earnings_per_share, dec!(1.35));
    }

    /// 頁面標題缺少年度（改版或載到錯誤頁）時必須整頁報錯——
    /// 年度是資料的 key，掛錯年度比沒抓到更糟。
    #[test]
    fn parse_profit_html_rejects_page_without_year() {
        let error = parse_profit_html("<html><body><p>維護中</p></body></html>").unwrap_err();
        assert!(error.to_string().contains("Failed to select"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 visit");

        match visit().await {
            Ok(e) => {
                tracing::debug!("{:#?}", e);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
