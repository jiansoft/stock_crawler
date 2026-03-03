//! # HiStock 即時報價採集
//!
//! 此模組負責透過 HiStock 的排行榜頁面取得台股即時報價資料。
//!
//! 使用端點：
//! - `stock/rank.aspx?p=all`：台股排行－全部資訊，包含所有上市櫃股票的即時行情。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
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

/// HiStock 即時報價快照。
#[derive(Debug, Clone)]
struct RealtimeSnapshot {
    /// 最新成交價。
    price: f64,
    /// 漲跌金額。
    change: f64,
    /// 漲跌幅（百分比）。
    change_range: f64,
}

/// 抓取並解析排行榜頁面，回傳所有股票的對照表。
async fn fetch_all_from_rank() -> Result<HashMap<String, RealtimeSnapshot>> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    let document = Html::parse_document(&body);
    
    let row_selector = Selector::parse("#CPHB1_gv tr")
        .map_err(|e| anyhow!("Failed to parse row selector: {:?}", e))?;
    let td_selector = Selector::parse("td").unwrap();
    
    let mut map = HashMap::new();

    for row in document.select(&row_selector) {
        let cols: Vec<_> = row.select(&td_selector).collect();
        if cols.len() < 5 {
            continue;
        }

        // 0: 股票代號, 1: 股票名稱, 2: 成交價, 3: 漲跌, 4: 幅度
        let symbol = cols[0].text().collect::<String>().trim().to_string();
        if symbol.is_empty() || !symbol.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let price_text = cols[2].text().collect::<String>();
        let change_text = cols[3].text().collect::<String>();
        let percent_text = cols[4].text().collect::<String>();

        let price = text::parse_f64(&price_text, None).unwrap_or(0.0);
        
        let is_negative = change_text.contains('▼');
        let mut change = text::parse_f64(&change_text, Some(vec!['▼', '▲'])).unwrap_or(0.0);
        if is_negative && change > 0.0 {
            change = -change;
        }

        let change_range = text::parse_f64(&percent_text, Some(vec!['%'])).unwrap_or(0.0);

        map.insert(symbol, RealtimeSnapshot {
            price,
            change,
            change_range,
        });
    }

    if map.is_empty() {
        return Err(anyhow!("Parsed HiStock rank page but found no data."));
    }

    Ok(map)
}

/// 取得指定股票的即時快照（每次呼叫皆重新抓取排行榜）。
async fn fetch_realtime_snapshot(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    let data = fetch_all_from_rank().await?;
    data.get(stock_symbol)
        .cloned()
        .ok_or_else(|| anyhow!("Stock symbol {} not found in HiStock rank page", stock_symbol))
}

#[async_trait]
impl StockInfo for HiStock {
    /// 取得指定股票的最新成交價。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let snapshot = fetch_realtime_snapshot(stock_symbol).await?;
        Decimal::from_f64_retain(snapshot.price)
            .ok_or_else(|| anyhow!("Failed to convert f64 to Decimal: {}", snapshot.price))
    }

    /// 取得指定股票的即時報價資訊。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let snapshot = fetch_realtime_snapshot(stock_symbol).await?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: snapshot.price,
            change: snapshot.change,
            change_range: snapshot.change_range,
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
                    "Failed to yahoo::get_stock_quotes because {:?}",
                    why
                ));
            }
        }
    }
}
