//! # HiStock 年度財報採集
//!
//! 此模組透過 HiStock 的「每股盈餘」頁面抓取歷年年度 EPS。
//!
//! 目前解析策略：
//! - 從頁面文字節點中找出 `季別/年度` 標頭列
//! - 再找出對應的 `總計` 列
//! - 以 `總計` 列作為各年度 EPS
//!
//! 由於此頁面未直接提供 `sales_per_share` 與 `profit_before_tax`，
//! 目前這兩個欄位暫時以 `0` 回填。

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    core::util::{self, text},
    infra::crawler::{
        histock::HOST,
        share::{self, AnnualProfitFetcher},
    },
};

/// HiStock 年度財報抓取器。
pub struct HiStockAnnualProfit {}

fn is_year_token(text: &str) -> bool {
    let normalized = text.trim_end_matches('-');
    normalized.len() == 4 && normalized.chars().all(|ch| ch.is_ascii_digit())
}

fn parse_year(text: &str) -> Result<i32> {
    text.trim_end_matches('-')
        .parse::<i32>()
        .map_err(|why| anyhow!("Failed to parse year '{}' because {:?}", text, why))
}

fn parse_eps(text: &str) -> Result<Decimal> {
    if text.trim() == "--" {
        return Ok(Decimal::ZERO);
    }

    text::parse_decimal(text, None)
}

fn parse_annual_profit_from_text_nodes(
    stock_symbol: &str,
    texts: &[String],
) -> Result<Vec<share::AnnualProfit>> {
    let header_idx = texts
        .iter()
        .position(|text| text == "季別/年度")
        .ok_or_else(|| anyhow!("Failed to find HiStock annual header row"))?;
    let total_idx = texts
        .iter()
        .position(|text| text == "總計")
        .ok_or_else(|| anyhow!("Failed to find HiStock annual total row"))?;

    let mut years = Vec::new();
    for text in texts.iter().skip(header_idx + 1) {
        if is_year_token(text) {
            years.push(parse_year(text)?);
            continue;
        }

        if !years.is_empty() {
            break;
        }
    }

    if years.is_empty() {
        return Err(anyhow!("Failed to parse HiStock annual year columns"));
    }

    let mut annual_profits = Vec::with_capacity(years.len());
    for (year, eps_text) in years.into_iter().zip(texts.iter().skip(total_idx + 1)) {
        let earnings_per_share = parse_eps(eps_text)?;
        annual_profits.push(share::AnnualProfit {
            stock_symbol: stock_symbol.to_string(),
            year,
            sales_per_share: Decimal::ZERO,
            earnings_per_share,
            profit_before_tax: Decimal::ZERO,
        });
    }

    Ok(annual_profits)
}

/// 解析 HiStock「每股盈餘」頁面的 HTML，取出各年度 EPS。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O。
/// 之所以從 [`visit`] 拆出來，是為了讓單元測試能用
/// `testdata/annual_profit.html` fixture 覆蓋「HTML → 文字節點 → 年度 EPS」
/// 的完整路徑；原本的 `parse_annual_profit_from_text_nodes` 測試只餵手工組的
/// 文字節點陣列，無法驗證前段的文字節點抽取（trim、過濾空白）是否正確。
///
/// # 解析流程
/// 1. 把整份 HTML 攤平成「非空白文字節點」序列——此頁面的表格結構層層巢狀，
///    直接以文字節點定位比維護一長串 CSS 選擇器來得穩定。
/// 2. 交給 [`parse_annual_profit_from_text_nodes`]：
///    找 `季別/年度` 標頭列取得年度欄，再找 `總計` 列逐欄對應出年度 EPS。
fn parse_annual_profit_html(stock_symbol: &str, html: &str) -> Result<Vec<share::AnnualProfit>> {
    let document = Html::parse_document(html);
    let texts = document
        .root_element()
        .text()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    parse_annual_profit_from_text_nodes(stock_symbol, &texts)
}

/// 抓取指定股票的年度財報資料。
///
/// 只負責 HTTP 抓取，解析工作交給 [`parse_annual_profit_html`]
/// （fetch/parse 分離，解析邏輯才能被 fixture 單元測試覆蓋）。
/// 目前以「每股盈餘」頁中的 `總計` 列作為年度 EPS。
pub async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
    let url = format!(
        "https://{host}/stock/{stock_symbol}/%E6%AF%8F%E8%82%A1%E7%9B%88%E9%A4%98",
        host = HOST,
        stock_symbol = stock_symbol
    );
    let html = util::http::get(&url, None).await?;
    parse_annual_profit_html(stock_symbol, &html)
}

#[async_trait]
impl AnnualProfitFetcher for HiStockAnnualProfit {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_annual_profit_from_text_nodes() {
        let texts = vec![
            "其他".to_string(),
            "季別/年度".to_string(),
            "2024".to_string(),
            "2023".to_string(),
            "2022".to_string(),
            "Q4".to_string(),
            "總計".to_string(),
            "1.16".to_string(),
            "3.25".to_string(),
            "--".to_string(),
        ];

        let result = parse_annual_profit_from_text_nodes("2838", &texts).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].year, 2024);
        assert_eq!(
            result[0].earnings_per_share,
            Decimal::from_str_exact("1.16").unwrap()
        );
        assert_eq!(result[1].year, 2023);
        assert_eq!(
            result[1].earnings_per_share,
            Decimal::from_str_exact("3.25").unwrap()
        );
        assert_eq!(result[2].year, 2022);
        assert_eq!(result[2].earnings_per_share, Decimal::ZERO);
    }

    /// 以貼近真實「每股盈餘」頁面形狀的 fixture 驗證完整解析路徑。
    ///
    /// 上面的 `test_parse_annual_profit_from_text_nodes` 餵的是手工組好的
    /// 文字節點陣列；這裡則從 HTML 開始走 [`parse_annual_profit_html`]，
    /// 連同前段的「HTML → 文字節點抽取」（trim、濾空白、頁面雜訊不干擾定位）
    /// 一併覆蓋。
    #[test]
    fn parse_annual_profit_html_parses_fixture() {
        // include_str! 的路徑相對於本檔案（histock/annual_profit.rs）→ histock/testdata/。
        const FIXTURE: &str = include_str!("testdata/annual_profit.html");

        let result = parse_annual_profit_html("2330", FIXTURE).unwrap();

        // 年度欄有三年：2025-（年度未結束、帶結尾連字號）、2024、2023。
        assert_eq!(result.len(), 3);

        // 「2025-」應正規化為 2025；其總計欄為「--」→ EPS 視為 0。
        assert_eq!(result[0].year, 2025);
        assert_eq!(result[0].earnings_per_share, Decimal::ZERO);

        assert_eq!(result[1].year, 2024);
        assert_eq!(
            result[1].earnings_per_share,
            Decimal::from_str_exact("45.25").unwrap()
        );
        assert_eq!(result[2].year, 2023);
        assert_eq!(
            result[2].earnings_per_share,
            Decimal::from_str_exact("32.34").unwrap()
        );

        // 此頁未提供每股營收與稅前淨利，依模組現行策略以 0 回填。
        assert_eq!(result[1].sales_per_share, Decimal::ZERO);
        assert_eq!(result[1].profit_before_tax, Decimal::ZERO);
        assert_eq!(result[1].stock_symbol, "2330");
    }

    /// 頁面缺少「季別/年度」標頭（例如改版或載到錯誤頁）時必須明確報錯，
    /// 讓呼叫端知道來源異常，而不是回傳空集合被誤當「沒有資料」。
    #[test]
    fn parse_annual_profit_html_rejects_unrelated_page() {
        let error =
            parse_annual_profit_html("2330", "<html><body><h1>404</h1></body></html>").unwrap_err();
        assert!(error.to_string().contains("annual header row"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 histock::annual_profit::visit");

        match visit("2330").await {
            Ok(result) => {
                dbg!(&result);
                tracing::debug!("histock : {:#?}", result);
            }
            Err(why) => {
                tracing::debug!("Failed to histock::annual_profit::visit because {:?}", why)
            }
        }

        tracing::debug!("結束 histock::annual_profit::visit");
    }
}
