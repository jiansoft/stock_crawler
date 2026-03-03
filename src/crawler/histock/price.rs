//! # HiStock 即時報價採集
//!
//! 此模組負責透過 HiStock 的排行榜頁面取得台股即時報價資料。
//!
//! 使用端點：
//! - `stock/rank.aspx?p=all`：台股排行－全部資訊，包含所有上市櫃股票的即時行情。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    crawler::{
        histock::{HiStock, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

/// 預編譯選擇器，避免重複解析 CSS 字串
#[allow(dead_code)]
static ROW_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("#CPHB1_gv tr").unwrap());
static TD_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());
static TR_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("tr").unwrap());

/// HiStock 即時報價快照。
#[derive(Debug, Clone)]
pub struct RealtimeSnapshot {
    /// 最新成交價。
    pub price: Decimal,
    /// 漲跌金額。
    pub change: Decimal,
    /// 漲跌幅（百分比）。
    pub change_range: Decimal,
}

/// 解析單一表格列資料。
/// 
/// 欄位索引 (Index):
/// 0: 股票代號, 1: 股票名稱, 2: 成交價, 3: 漲跌, 4: 幅度
fn parse_row(row: scraper::element_ref::ElementRef) -> Option<(String, RealtimeSnapshot)> {
    let mut tds = row.select(&TD_SELECTOR);
    
    // 0: 股票代號
    let symbol = tds.next()?.text().collect::<String>().trim().to_string();
    if symbol.is_empty() || !symbol.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    // 1: 跳過名稱
    let _name = tds.next()?;
    
    // 2: 成交價
    let price_text = tds.next()?.text().collect::<String>();
    let price = text::parse_decimal(&price_text, None).unwrap_or(Decimal::ZERO);
    
    // 3: 漲跌
    let change_node = tds.next()?;
    let change_text = change_node.text().collect::<String>();
    let is_negative = change_text.contains('▼');
    let mut change = text::parse_decimal(&change_text, Some(vec!['▼', '▲', '-'])).unwrap_or(Decimal::ZERO);
    if is_negative && change > Decimal::ZERO {
        change = -change;
    }

    // 4: 幅度
    let percent_text = tds.next()?.text().collect::<String>();
    let change_range = text::parse_decimal(&percent_text, Some(vec!['%', '-'])).unwrap_or(Decimal::ZERO);

    Some((symbol, RealtimeSnapshot {
        price,
        change,
        change_range,
    }))
}

/// 抓取並解析排行榜頁面，回傳所有股票的對照表。
#[allow(dead_code)]
pub async fn fetch_all_from_rank() -> Result<HashMap<String, RealtimeSnapshot>> {

    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    let document = Html::parse_document(&body);
    
    let mut map = HashMap::with_capacity(1200);

    for row in document.select(&ROW_SELECTOR) {
        if let Some((symbol, snapshot)) = parse_row(row) {
            map.insert(symbol, snapshot);
        }
    }

    if map.is_empty() {
        return Err(anyhow!("Parsed HiStock rank page but found no data."));
    }

    Ok(map)
}

/// 取得指定股票的即時快照。
/// 
/// 優化策略：
/// 1. 使用正則表達式從 2.5MB 的 HTML 中「切出」目標股票的那一行。
/// 2. 只對該行 HTML 碎片進行 DOM 解析，大幅降低記憶體與 CPU 負載。
async fn fetch_single_from_rank(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    
    // 使用正則表達式快速定位目標行
    // 放寬匹配規則以支援 <tr class="..."> 與標籤內的空白
    let pattern = format!(r"(?is)<tr[^>]*?>\s*<td[^>]*?>\s*{}\s*</td>.*?</tr>", stock_symbol);
    let re = Regex::new(&pattern)?;
    
    if let Some(mat) = re.find(&body) {
        let row_html = mat.as_str();
        // 為了讓 scraper 能正確解析單一行，我們將其封裝在 table 標籤中
        let fragment_html = format!("<table>{}</table>", row_html);
        let fragment = Html::parse_fragment(&fragment_html);
        
        // 解析該行碎片
        if let Some(row_ref) = fragment.select(&TR_SELECTOR).next() {
            if let Some((_, snapshot)) = parse_row(row_ref) {
                return Ok(snapshot);
            }
        }
    }

    Err(anyhow!("Stock symbol {} not found in HiStock rank page", stock_symbol))
}

#[async_trait]
impl StockInfo for HiStock {
    /// 取得指定股票的最新成交價。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let snapshot = fetch_single_from_rank(stock_symbol).await?;
        Ok(snapshot.price)
    }

    /// 取得指定股票的即時報價資訊。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let snapshot = fetch_single_from_rank(stock_symbol).await?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: snapshot.price.to_f64().unwrap_or(0.0),
            change: snapshot.change.to_f64().unwrap_or(0.0),
            change_range: snapshot.change_range.to_f64().unwrap_or(0.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 HiStock::get_stock_price".to_string());

        match HiStock::get_stock_price("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("HiStock::get_stock_price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }
    }

    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 HiStock::get_stock_quotes".to_string());

        match HiStock::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("HiStock::get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to HiStock::get_stock_quotes because {:?}",
                    why
                ));
            }
        }
    }
}
