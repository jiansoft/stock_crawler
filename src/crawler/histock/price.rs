//! # HiStock 即時報價採集 (全市場快取版)
//!
//! 此模組負責透過 HiStock 的排行榜頁面取得台股即時報價資料。
//!
//! ## 核心設計：全市場智慧快取
//! 1. **全欄位快取**：包含代號、名稱、成交、漲跌、幅、開盤、最高、最低、昨收、成交量。
//! 2. **智慧啟停**：
//!    - 在開盤期間，每 10 秒自動抓取全市場排行榜並更新快取。
//!    - 非交易時間停止任務並清空快取。
//! 3. **容錯解析**：正確處理 '--' 符號（視為 0），確保所有股票都能進入快取。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use tokio::sync::RwLock;
use tokio::time::sleep;

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

/// 全域快取狀態
static CACHE_DATA: Lazy<RwLock<HashMap<String, RealtimeSnapshot>>> = Lazy::new(|| RwLock::new(HashMap::with_capacity(1200)));
static IS_CACHING: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// HiStock 即時報價快照（完整版）。
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeSnapshot {
    pub symbol: String,
    pub name: String,
    pub price: Decimal,
    pub change: Decimal,
    pub change_range: Decimal,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub last_close: Decimal,
    pub volume: Decimal, // 單位：張
}

/// 解析單一表格列資料。
/// 
/// 欄位索引 (根據 HiStock rank.aspx?p=all):
/// 0:代號, 1:名稱, 2:成交, 3:漲跌, 4:幅, 5:周漲跌, 6:振幅, 7:開盤, 8:最高, 9:最低, 10:昨收, 11:成交量
fn parse_row(row: scraper::element_ref::ElementRef) -> Result<Option<(String, RealtimeSnapshot)>> {
    let mut tds = row.select(&TD_SELECTOR);
    
    // 0: 股票代號
    let symbol_node = match tds.next() {
        Some(node) => node,
        None => return Ok(None),
    };
    let symbol = symbol_node.text().collect::<String>().trim().to_string();
    if symbol.is_empty() || !symbol.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }

    // 1: 股票名稱
    let name = tds.next().context("Missing name")?.text().collect::<String>().trim().to_string();
    
    // 輔助解析函式：處理 -- 或解析失敗的情況
    let parse_val = |node: Option<scraper::element_ref::ElementRef>| -> Decimal {
        let t = node.map(|n| n.text().collect::<String>()).unwrap_or_default();
        if t.contains("--") || t.trim().is_empty() {
            Decimal::ZERO
        } else {
            text::parse_decimal(&t, None).unwrap_or(Decimal::ZERO)
        }
    };

    // 2: 成交價
    let price = parse_val(tds.next());
    
    // 3: 漲跌 (特殊處理 ▲/▼)
    let change_node = tds.next();
    let change_text = change_node.map(|n| n.text().collect::<String>()).unwrap_or_default();
    let mut change = Decimal::ZERO;
    let mut is_negative = false;
    if !change_text.contains("--") && !change_text.trim().is_empty() {
        is_negative = change_text.contains('▼');
        change = text::parse_decimal(&change_text, Some(vec!['▼', '▲', ' ', '+'])).unwrap_or(Decimal::ZERO);
        if is_negative && change > Decimal::ZERO { change = -change; }
    }

    // 4: 漲跌幅
    let range_node = tds.next();
    let range_text = range_node.map(|n| n.text().collect::<String>()).unwrap_or_default();
    let mut change_range = Decimal::ZERO;
    if !range_text.contains("--") && !range_text.trim().is_empty() {
        change_range = text::parse_decimal(&range_text, Some(vec!['%', ' ', '+', '▼', '▲'])).unwrap_or(Decimal::ZERO);
        if is_negative && change_range > Decimal::ZERO { change_range = -change_range; }
    }

    // 5: 周漲跌 (跳過)
    tds.next(); 
    // 6: 振幅 (跳過)
    tds.next(); 

    // 7: 開盤
    let open = parse_val(tds.next());
    // 8: 最高
    let high = parse_val(tds.next());
    // 9: 最低
    let low = parse_val(tds.next());
    // 10: 昨收
    let last_close = parse_val(tds.next());
    // 11: 成交量 (張)
    let volume = parse_val(tds.next());

    Ok(Some((symbol.clone(), RealtimeSnapshot {
        symbol,
        name,
        price,
        change,
        change_range,
        open,
        high,
        low,
        last_close,
        volume,
    })))
}

/// 啟動 10 秒定時快取任務
pub fn start_caching_task() {
    if IS_CACHING.load(Ordering::SeqCst) {
        return;
    }
    IS_CACHING.store(true, Ordering::SeqCst);

    tokio::spawn(async move {
        crate::logging::info_file_async("HiStock 全市場快取任務啟動".to_string());
        
        while IS_CACHING.load(Ordering::SeqCst) {
            let start_time = std::time::Instant::now();
            match fetch_all_from_rank().await {
                Ok(new_data) => {
                    let count = new_data.len();
                    let mut cache = CACHE_DATA.write().await;
                    *cache = new_data;
                    crate::logging::debug_file_async(format!(
                        "HiStock 快取已更新，共 {} 檔股票，耗時 {:?}", 
                        count, start_time.elapsed()
                    ));
                }
                Err(e) => {
                    crate::logging::error_file_async(format!("HiStock 快取更新失敗: {:?}", e));
                }
            }
            sleep(Duration::from_secs(10)).await;
        }
        crate::logging::info_file_async("HiStock 快取任務已停止".to_string());
    });
}

/// 停止定時快取任務並清空快取
pub async fn stop_caching_task() {
    IS_CACHING.store(false, Ordering::SeqCst);
    let mut cache = CACHE_DATA.write().await;
    cache.clear();
}

/// 抓取全量資料
async fn fetch_all_from_rank() -> Result<HashMap<String, RealtimeSnapshot>> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    let document = Html::parse_document(&body);
    
    let row_selector = Selector::parse("#CPHB1_gv tr").unwrap();
    let mut map = HashMap::with_capacity(1200);

    for row in document.select(&row_selector) {
        if let Some((symbol, snapshot)) = parse_row(row)? {
            map.insert(symbol, snapshot);
        }
    }
    
    if map.is_empty() {
        return Err(anyhow!("Failed to parse HiStock rank page (empty map)"));
    }
    
    Ok(map)
}

/// 取得指定股票的即時快照 (優先使用快取，否則即時碎片抓取)
async fn get_snapshot(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    {
        let cache = CACHE_DATA.read().await;
        if let Some(s) = cache.get(stock_symbol) {
            return Ok(s.clone());
        }
    }
    fetch_single_from_rank(stock_symbol).await
}

/// 即時碎片抓取邏輯 (維持高效定位)
async fn fetch_single_from_rank(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    
    let target_tag = format!(">{}</td>", stock_symbol);
    if let Some(pos) = body.find(&target_tag) {
        let start_of_tr = body[..pos].rfind("<tr").unwrap_or(0);
        let end_of_tr = body[pos..].find("</tr>").map(|e| pos + e + 5).unwrap_or(body.len());
        
        let row_html = &body[start_of_tr..end_of_tr];
        let fragment_html = format!("<table>{}</table>", row_html);
        let fragment = Html::parse_fragment(&fragment_html);
        
        if let Some(row_ref) = fragment.select(&TR_SELECTOR).next() {
            if let Some((_, snapshot)) = parse_row(row_ref)? {
                return Ok(snapshot);
            }
        }
    }

    Err(anyhow!("Stock symbol {} not found", stock_symbol))
}

#[async_trait]
impl StockInfo for HiStock {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let snapshot = get_snapshot(stock_symbol).await?;
        Ok(snapshot.price)
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let snapshot = get_snapshot(stock_symbol).await?;

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
    use rust_decimal_macros::dec;
    use crate::logging;

    #[test]
    fn test_parse_full_row_from_user_sample() {
        let html = r#"<table><tr class="alt-row">
			<td>5274</td><td>
                                   <a href="/stock/5274" target="_blank">信驊</a>
                               </td><td>
                                   <span id="CPHB1_gv_lbDeal_0" class="price-down">9445</span>
                               </td><td>
                                   <span id="CPHB1_gv_lbChange_0" class="price-down">▼-55.00</span>
                               </td><td>
                                   <span id="CPHB1_gv_lbPercentage_0" class="price-down">-0.58%</span>
                               </td><td>
                                   <span id="CPHB1_gv_lbWeekChange_0" class="price-down">-3.18%</span>
                               </td><td>2.95%</td><td>9620</td><td>
                                   <span id="CPHB1_gv_lbHigh_0">9715</span>
                               </td><td>
                                   <span id="CPHB1_gv_lbLow_0">9435</span>
                               </td><td>9500</td><td>44</td><td>4.156</td>
		</tr></table>"#;
        let fragment = Html::parse_fragment(html);
        let row = fragment.select(&TR_SELECTOR).next().unwrap();
        let (symbol, snapshot) = parse_row(row).unwrap().unwrap();
        
        assert_eq!(symbol, "5274");
        assert_eq!(snapshot.name, "信驊");
        assert_eq!(snapshot.price, dec!(9445));
        assert_eq!(snapshot.change, dec!(-55));
        assert_eq!(snapshot.change_range, dec!(-0.58));
        assert_eq!(snapshot.open, dec!(9620));
        assert_eq!(snapshot.high, dec!(9715));
        assert_eq!(snapshot.low, dec!(9435));
        assert_eq!(snapshot.last_close, dec!(9500));
        assert_eq!(snapshot.volume, dec!(44));
    }

    #[test]
    fn test_parse_no_change_row() {
        let html = r#"<table><tr><td>6584</td><td>南俊國際</td><td><span id="lbDeal">425</span></td><td><span>--</span></td><td><span>--</span></td><td>...</td><td>...</td><td>423.5</td><td>435</td><td>423.5</td><td>425</td><td>148</td><td>0.629</td></tr></table>"#;
        let fragment = Html::parse_fragment(html);
        let row = fragment.select(&TR_SELECTOR).next().unwrap();
        let (symbol, snapshot) = parse_row(row).unwrap().unwrap();
        
        assert_eq!(symbol, "6584");
        assert_eq!(snapshot.change, Decimal::ZERO);
        assert_eq!(snapshot.change_range, Decimal::ZERO);
        assert_eq!(snapshot.volume, dec!(148));
    }

    #[tokio::test]
    async fn test_cache_mechanism() {
        // 1. 清空快取確保環境乾淨
        stop_caching_task().await;
        {
            let cache = CACHE_DATA.read().await;
            assert!(cache.is_empty(), "Cache should be empty after stop_caching_task");
        }

        // 2. 模擬放入快取資料
        let mock_symbol = "MOCK99";
        let mock_snapshot = RealtimeSnapshot {
            symbol: mock_symbol.to_string(),
            name: "測試股".to_string(),
            price: dec!(100.5),
            change: dec!(1.5),
            change_range: dec!(1.51),
            open: dec!(99.0),
            high: dec!(101.0),
            low: dec!(98.5),
            last_close: dec!(99.0),
            volume: dec!(500),
        };

        {
            let mut cache = CACHE_DATA.write().await;
            cache.insert(mock_symbol.to_string(), mock_snapshot.clone());
        }

        // 3. 測試 get_snapshot 是否優先從快取取得
        let result = get_snapshot(mock_symbol).await.unwrap();
        assert_eq!(result, mock_snapshot, "Should retrieve snapshot from cache");

        // 4. 測試停止任務是否會清空快取
        stop_caching_task().await;
        {
            let cache = CACHE_DATA.read().await;
            assert!(cache.is_empty(), "Cache should be cleared after stopping");
            assert!(!IS_CACHING.load(Ordering::SeqCst), "IS_CACHING should be false");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_live_rank_data() {
        // 測試抓取全量資料
        let result = fetch_all_from_rank().await.unwrap();
        assert!(!result.is_empty(), "Should fetch at least some stocks from HiStock");
        
        // 檢查台積電 (2330) 是否在其中
        if let Some(tsmc) = result.get("2330") {
            println!("Live TSMC Snapshot: {:?}", tsmc);
            assert_eq!(tsmc.name, "台積電");
            assert!(tsmc.price > Decimal::ZERO);
        } else {
            panic!("TSMC (2330) not found in live rank data");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_live_single_stock() {
        // 測試單一股票碎片抓取 (不靠快取)
        let tsmc = fetch_single_from_rank("2330").await.unwrap();
        println!("Live Single TSMC: {:?}", tsmc);
        assert_eq!(tsmc.symbol, "2330");
        assert_eq!(tsmc.name, "台積電");
        assert!(tsmc.price > Decimal::ZERO);
    }


    #[tokio::test]
    #[ignore]
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
    #[ignore]
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
    #[tokio::test]
    #[ignore]
    async fn test_get_stock_price_with_cache_verification() {
        // 1. 清空快取
        stop_caching_task().await;

        // 2. 抓取全量並填充快取
        let all_stocks = fetch_all_from_rank().await.unwrap();
        {
            let mut cache = CACHE_DATA.write().await;
            *cache = all_stocks;
        }

        // 3. 測試透過公用介面取得價格 (此時應從快取秒讀)
        let price = HiStock::get_stock_price("2330").await.unwrap();
        assert!(price > Decimal::ZERO);
        println!("Verified Price from Cache: {}", price);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_stock_quotes_with_cache_verification() {
        // 1. 清空快取
        stop_caching_task().await;

        // 2. 抓取全量並填充快取
        let all_stocks = fetch_all_from_rank().await.unwrap();
        {
            let mut cache = CACHE_DATA.write().await;
            *cache = all_stocks;
        }

        // 3. 測試透過公用介面取得報價物件
        let quotes = HiStock::get_stock_quotes("2330").await.unwrap();
        assert_eq!(quotes.stock_symbol, "2330");
        assert!(quotes.price > 0.0);
        println!("Verified Quotes from Cache: {:?}", quotes);
    }
}
