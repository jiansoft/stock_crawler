//! # HiStock 即時報價採集
//!
//! 此模組負責透過 HiStock 的排行榜頁面取得台股即時報價資料。
//!
//! ## 核心設計：混合解析策略 (Hybrid Parsing)
//! 為了處理 HiStock 排行榜巨大的 HTML 頁面 (約 2.5MB)，此模組採用以下策略優化效能：
//! 1. **高效定位**：先透過字串搜尋找出目標股票的 `<td>` 區塊位置。
//! 2. **碎片解析**：僅針對目標股票所在的 `<tr>...</tr>` HTML 碎片進行 DOM 解析。
//! 3. **全精確度**：使用 `Decimal` 進行全鏈路運算，避免浮點數誤差。
//!
//! 這種方式避免了解析整個 2.5MB DOM 樹帶來的巨大記憶體與 CPU 開銷。

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
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

/// 預編譯碎片解析所需的選擇器
static TD_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());
static TR_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("tr").unwrap());

/// HiStock 即時報價快照。
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeSnapshot {
    pub price: Decimal,
    pub change: Decimal,
    pub change_range: Decimal,
}

/// 解析單一表格列資料。
/// 
/// 欄位索引 (Index):
/// 0: 股票代號, 1: 股票名稱, 2: 成交價, 3: 漲跌, 4: 幅度
fn parse_row(row: scraper::element_ref::ElementRef) -> Result<Option<(String, RealtimeSnapshot)>> {
    let mut tds = row.select(&TD_SELECTOR);
    
    // 0: 股票代號
    let symbol_node = tds.next().context("Missing symbol column")?;
    let symbol = symbol_node.text().collect::<String>().trim().to_string();
    if symbol.is_empty() || !symbol.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }

    // 1: 股票名稱 (跳過)
    tds.next().context("Missing name column")?;
    
    // 2: 成交價
    let price_text = tds.next().context("Missing price column")?.text().collect::<String>();
    let price = text::parse_decimal(&price_text, None)
        .map_err(|e| anyhow!("Failed to parse price '{}': {}", price_text, e))?;
    
    // 3: 漲跌 (判斷正負號)
    let change_text = tds.next().context("Missing change column")?.text().collect::<String>();
    let is_negative = change_text.contains('▼');
    let mut change = text::parse_decimal(&change_text, Some(vec!['▼', '▲', ' ', '+']))
        .map_err(|e| anyhow!("Failed to parse change '{}': {}", change_text, e))?;
    
    if is_negative && change > Decimal::ZERO {
        change = -change;
    }

    // 4: 幅度 (百分比)
    let percent_text = tds.next().context("Missing range column")?.text().collect::<String>();
    let mut change_range = text::parse_decimal(&percent_text, Some(vec!['%', ' ', '+', '▼', '▲']))
        .map_err(|e| anyhow!("Failed to parse range '{}': {}", percent_text, e))?;
    
    if is_negative && change_range > Decimal::ZERO {
        change_range = -change_range;
    }

    Ok(Some((symbol, RealtimeSnapshot {
        price,
        change,
        change_range,
    })))
}

/// 取得指定股票的即時快照。
/// 
/// 採用字串定位優化：不解析整頁 DOM，直接切出目標行。
async fn fetch_single_from_rank(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    
    // 高效定位目標股票所在的 <td>
    let target_tag = format!(">{}</td>", stock_symbol);
    if let Some(pos) = body.find(&target_tag) {
        let start_of_tr = body[..pos].rfind("<tr").unwrap_or(0);
        let end_of_tr = body[pos..].find("</tr>")
            .map(|e| pos + e + 5)
            .unwrap_or(body.len());
        
        let row_html = &body[start_of_tr..end_of_tr];
        let fragment_html = format!("<table>{}</table>", row_html);
        let fragment = Html::parse_fragment(&fragment_html);
        
        if let Some(row_ref) = fragment.select(&TR_SELECTOR).next() {
            if let Some((_, snapshot)) = parse_row(row_ref)? {
                return Ok(snapshot);
            }
        }
    }

    Err(anyhow!("Stock symbol {} not found in HiStock rank page", stock_symbol))
}

#[async_trait]
impl StockInfo for HiStock {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let snapshot = fetch_single_from_rank(stock_symbol).await?;
        Ok(snapshot.price)
    }

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
