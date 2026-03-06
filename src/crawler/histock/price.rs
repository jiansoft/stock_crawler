//! # HiStock 即時報價採集 (全市場快取版)
//!
//! 此模組負責透過 HiStock 的排行榜頁面取得台股即時報價資料。
//!
//! ## 核心設計：全市場智慧快取
//! 1. **全欄位快取**：包含代號、名稱、成交、漲跌、幅、開盤、最高、最低、昨收、成交量。
//! 2. **外部驅動啟停**：
//!    - 依賴外部事件 (如 `src/event/trace/stock_price.rs`) 在開盤期間啟動定時任務。
//!    - 依賴收盤事件停止任務。停止後會清空快取以節省記憶體並確保下次啟動時資料新鮮。
//! 3. **消費者共用**：背景任務更新後的資料會寫入 [`SHARE`](crate::cache::SHARE)
//!    的 `stock_snapshots`，供追蹤任務與 `StockInfo` 介面共用。
//! 4. **嚴格解析**：不容忍損壞或格式錯誤的報價，解析失敗會傳回錯誤而非默默變 0。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::{
    cache::{RealtimeSnapshot, SHARE},
    crawler::{
        histock::{HiStock, HOST},
        StockInfo,
    },
    declare,
    event::trace::price_tasks as trace_price_tasks,
    util::{self, text},
};

/// 預編譯所需的選擇器
static TD_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());
static ROW_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("#CPHB1_gv tr").unwrap());

/// 全域快取狀態
static IS_CACHING: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// 用於解決 Single-flight (重複抓取) 的互斥鎖
static FETCH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// 解析單一表格列資料。
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
    let name = tds
        .next()
        .context("Missing name")?
        .text()
        .collect::<String>()
        .trim()
        .to_string();

    // 輔助解析函式：嚴格處理數值解析，不再默默變 0
    let parse_val =
        |node: Option<scraper::element_ref::ElementRef>, field_name: &str| -> Result<Decimal> {
            let t = node
                .map(|n| n.text().collect::<String>())
                .unwrap_or_default();
            let t = t.trim();
            if t == "--" || t.is_empty() {
                Ok(Decimal::ZERO)
            } else {
                text::parse_decimal(t, None)
                    .map_err(|e| anyhow!("Failed to parse {} for {}: {:?}", field_name, symbol, e))
            }
        };

    let price = parse_val(tds.next(), "price")?;

    // 漲跌與幅 (處理符號與趨勢)
    let change_node = tds.next();
    let change_text = change_node
        .map(|n| n.text().collect::<String>())
        .unwrap_or_default();
    let mut change = Decimal::ZERO;
    let mut is_negative = false;
    if !change_text.contains("--") && !change_text.trim().is_empty() {
        is_negative = change_text.contains('▼');
        change = text::parse_decimal(&change_text, Some(vec!['▼', '▲', ' ', '+']))
            .map_err(|e| anyhow!("Failed to parse change for {}: {:?}", symbol, e))?;
        if is_negative && change > Decimal::ZERO {
            change = -change;
        }
    }

    let range_node = tds.next();
    let range_text = range_node
        .map(|n| n.text().collect::<String>())
        .unwrap_or_default();
    let mut change_range = Decimal::ZERO;
    if !range_text.contains("--") && !range_text.trim().is_empty() {
        change_range = text::parse_decimal(&range_text, Some(vec!['%', ' ', '+', '▼', '▲']))
            .map_err(|e| anyhow!("Failed to parse change_range for {}: {:?}", symbol, e))?;
        if is_negative && change_range > Decimal::ZERO {
            change_range = -change_range;
        }
    }

    // 跳過 5: 周漲跌, 6: 振幅
    tds.next();
    tds.next();

    let open = parse_val(tds.next(), "open")?;
    let high = parse_val(tds.next(), "high")?;
    let low = parse_val(tds.next(), "low")?;
    let last_close = parse_val(tds.next(), "last_close")?;
    let volume = parse_val(tds.next(), "volume")?;

    // 使用 new 方法強制填入必要欄位，其餘欄位則個別設定
    let mut snapshot = RealtimeSnapshot::new(symbol.clone(), price);
    snapshot.name = name;
    snapshot.change = change;
    snapshot.change_range = change_range;
    snapshot.open = open;
    snapshot.high = high;
    snapshot.low = low;
    snapshot.last_close = last_close;
    snapshot.volume = volume;

    Ok(Some((symbol, snapshot)))
}

/// 比對新舊快取，收集「價格實際有異動」的股票清單。
///
/// # 回傳
/// - `Vec<(String, Decimal)>`：股票代號與最新成交價的配對。
///
/// # 行為
/// - 若舊快取中不存在該股票，視為新價格事件。
/// - 若舊快取中的 `price` 與新資料相同，則不產生事件。
/// - 價格為 0 的資料不會發出事件，避免無效資料觸發追蹤判斷。
fn collect_changed_price_updates(
    new_data: &HashMap<String, RealtimeSnapshot>,
) -> Vec<(String, Decimal)> {
    let old_cache = SHARE.stock_snapshots.read().ok();
    let mut updates = Vec::new();

    for (symbol, snapshot) in new_data {
        if snapshot.price == Decimal::ZERO {
            continue;
        }

        let has_changed = old_cache
            .as_ref()
            .and_then(|cache| cache.get(symbol))
            .is_none_or(|old_snapshot| old_snapshot.price != snapshot.price);

        if has_changed {
            updates.push((symbol.clone(), snapshot.price));
        }
    }

    updates
}

/// 啟動定時快取任務。
///
/// 此任務會固定重新抓取 HiStock 全市場排行榜，並以全量覆蓋方式更新
/// [`SHARE`](crate::cache::SHARE) 的即時報價快取。
///
/// 若任務已在執行中，重複呼叫不會再額外啟動第二個背景迴圈。
pub fn start_caching_task() {
    if IS_CACHING.load(Ordering::SeqCst) {
        return;
    }
    IS_CACHING.store(true, Ordering::SeqCst);

    tokio::spawn(async move {
        crate::logging::info_file_async("HiStock 全市場快取任務啟動".to_string());

        while IS_CACHING.load(Ordering::SeqCst) {
            let start_time = std::time::Instant::now();

            // 背景任務也受 FETCH_LOCK 控制，但優先讓外部請求先行
            let result = {
                let _lock = FETCH_LOCK.lock().await;
                fetch_all_from_rank().await
            };

            match result {
                Ok(new_data) => {
                    let count = new_data.len();
                    let price_updates = collect_changed_price_updates(&new_data);
                    SHARE.set_stock_snapshots(new_data);
                    trace_price_tasks::publish_price_updates(price_updates);
                    crate::logging::debug_file_async(format!(
                        "HiStock 快取已更新，共 {} 檔股票，耗時 {:?}",
                        count,
                        start_time.elapsed()
                    ));
                }
                Err(e) => {
                    crate::logging::error_file_async(format!("HiStock 快取更新失敗: {:?}", e));
                }
            }

            if !IS_CACHING.load(Ordering::SeqCst) {
                break;
            }
            sleep(Duration::from_secs(5)).await;
        }
        crate::logging::info_file_async("HiStock 快取任務已停止".to_string());
    });
}

/// 停止定時快取任務並清空即時報價快取。
///
/// 清空快取的目的，是避免收盤後保留過時盤中資料，讓下次開盤重新暖機時
/// 一定從新資料開始。
pub async fn stop_caching_task() {
    IS_CACHING.store(false, Ordering::SeqCst);
    SHARE.clear_stock_snapshots();
}

/// 從 HiStock 排行榜抓取全市場即時報價資料。
///
/// # 回傳
/// - `Ok(HashMap<String, RealtimeSnapshot>)`：以股票代號為 key 的完整即時快照。
/// - `Err(_)`：HTTP 抓取失敗、HTML 結構異常，或解析後完全沒有有效資料。
async fn fetch_all_from_rank() -> Result<HashMap<String, RealtimeSnapshot>> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    let document = Html::parse_document(&body);

    let mut map = HashMap::with_capacity(1200);
    for row in document.select(&ROW_SELECTOR) {
        if let Some((symbol, snapshot)) = parse_row(row)? {
            map.insert(symbol, snapshot);
        }
    }

    if map.is_empty() {
        return Err(anyhow!("Failed to parse HiStock rank page (empty map)"));
    }
    Ok(map)
}

/// 取得指定股票的即時快照。
///
/// 讀取策略如下：
/// 1. 先直接查詢全市場快取。
/// 2. 若快取未命中，使用 single-flight 鎖避免多個呼叫端同時觸發全量重抓。
/// 3. 抓取成功後，以新資料覆蓋全市場快取，再回傳目標股票快照。
async fn get_snapshot(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    // 第一階段：嘗試取得快取
    if let Some(s) = SHARE.get_stock_snapshot(stock_symbol) {
        return Ok(s);
    }

    // 第二階段：快取失效，使用互斥鎖防止重複抓取 (Single-flight)
    let _lock = FETCH_LOCK.lock().await;

    // 拿到鎖後再檢查一次快取，可能剛才有人抓過了
    if let Some(s) = SHARE.get_stock_snapshot(stock_symbol) {
        return Ok(s);
    }

    crate::logging::info_file_async(format!("HiStock 快取失效 ({})，觸發全量抓取", stock_symbol));
    let new_data = fetch_all_from_rank().await?;

    let snapshot = new_data.get(stock_symbol).cloned().ok_or_else(|| {
        anyhow!(
            "Stock symbol {} not found in HiStock rank page after refresh",
            stock_symbol
        )
    })?;

    // 更新全量快取
    let price_updates = collect_changed_price_updates(&new_data);
    SHARE.set_stock_snapshots(new_data);
    trace_price_tasks::publish_price_updates(price_updates);

    Ok(snapshot)
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
            price: snapshot
                .price
                .to_f64()
                .context("Decimal to f64 conversion failed (price)")?,
            change: snapshot
                .change
                .to_f64()
                .context("Decimal to f64 conversion failed (change)")?,
            change_range: snapshot
                .change_range
                .to_f64()
                .context("Decimal to f64 conversion failed (range)")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;
    use rust_decimal_macros::dec;

    /// 驗證全量快取更新前，只會針對價格實際異動的股票產生價格事件。
    #[test]
    fn test_collect_changed_price_updates() {
        SHARE.clear_stock_snapshots();

        let mut existing_snapshot = RealtimeSnapshot::new("2330".to_string(), dec!(998));
        existing_snapshot.name = "台積電".to_string();
        let mut cache = HashMap::new();
        cache.insert("2330".to_string(), existing_snapshot);
        SHARE.set_stock_snapshots(cache);

        let mut new_data = HashMap::new();
        new_data.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(1000)),
        );
        new_data.insert(
            "2317".to_string(),
            RealtimeSnapshot::new("2317".to_string(), dec!(180)),
        );
        new_data.insert(
            "2454".to_string(),
            RealtimeSnapshot::new("2454".to_string(), Decimal::ZERO),
        );

        let mut updates = collect_changed_price_updates(&new_data);
        updates.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(
            updates,
            vec![
                ("2317".to_string(), dec!(180)),
                ("2330".to_string(), dec!(1000)),
            ]
        );

        SHARE.clear_stock_snapshots();
    }

    #[test]
    fn test_parse_full_row_from_user_sample() {
        let html = r#"<table><tr class="alt-row">
			<td>5274</td><td>信驊</td><td><span class="price-down">9445</span></td>
            <td><span class="price-down">▼-55.00</span></td><td><span class="price-down">-0.58%</span></td>
            <td>-3.18%</td><td>2.95%</td><td>9620</td><td>9715</td><td>9435</td><td>9445</td><td>44</td><td>4.156</td>
		</tr></table>"#;
        let fragment = Html::parse_fragment(html);
        let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
        let row = fragment.select(&tr_selector).next().unwrap();
        let (symbol, snapshot) = parse_row(row).unwrap().unwrap();

        assert_eq!(symbol, "5274");
        assert_eq!(snapshot.name, "信驊");
        assert_eq!(snapshot.price, dec!(9445));
        assert_eq!(snapshot.change, dec!(-55));
    }

    #[test]
    fn test_parse_no_change_row() {
        let html = r#"<table><tr><td>6584</td><td>南俊國際</td><td>425</td><td>--</td><td>--</td><td>...</td><td>...</td><td>423.5</td><td>435</td><td>423.5</td><td>425</td><td>148</td><td>0.629</td></tr></table>"#;
        let fragment = Html::parse_fragment(html);
        let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
        let row = fragment.select(&tr_selector).next().unwrap();
        let (symbol, snapshot) = parse_row(row).unwrap().unwrap();

        assert_eq!(symbol, "6584");
        assert_eq!(snapshot.change, Decimal::ZERO);
        assert_eq!(snapshot.volume, dec!(148));
    }

    #[tokio::test]
    async fn test_cache_mechanism() {
        stop_caching_task().await;
        {
            assert!(
                SHARE.stock_snapshots.read().unwrap().is_empty(),
                "Cache should be empty after stop_caching_task"
            );
        }

        let mock_symbol = "MOCK99";
        let mut mock_snapshot = RealtimeSnapshot::new(mock_symbol.to_string(), dec!(100.5));
        mock_snapshot.name = "測試股".to_string();
        mock_snapshot.change = dec!(1.5);
        mock_snapshot.change_range = dec!(1.51);
        mock_snapshot.open = dec!(99.0);
        mock_snapshot.high = dec!(101.0);
        mock_snapshot.low = dec!(98.5);
        mock_snapshot.last_close = dec!(99.0);
        mock_snapshot.volume = dec!(500);

        {
            let mut cache = SHARE.stock_snapshots.write().unwrap();
            cache.insert(mock_symbol.to_string(), mock_snapshot.clone());
        }

        let result = get_snapshot(mock_symbol).await.unwrap();
        assert_eq!(result, mock_snapshot);

        stop_caching_task().await;
        {
            assert!(SHARE.stock_snapshots.read().unwrap().is_empty());
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_start_caching_task_integration() {
        stop_caching_task().await;
        start_caching_task();

        println!("Waiting for background fetch...");
        tokio::time::sleep(Duration::from_secs(6)).await;

        {
            let cache = SHARE.stock_snapshots.read().unwrap();
            assert!(!cache.is_empty());
            println!("Background cache populated with {} stocks", cache.len());
        }

        let price = HiStock::get_stock_price("2330").await.unwrap();
        assert!(price > Decimal::ZERO);

        stop_caching_task().await;
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
        SHARE.set_stock_snapshots(all_stocks);

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
        SHARE.set_stock_snapshots(all_stocks);

        // 3. 測試透過公用介面取得報價物件
        let quotes = HiStock::get_stock_quotes("2330").await.unwrap();
        assert_eq!(quotes.stock_symbol, "2330");
        assert!(quotes.price > 0.0);
        println!("Verified Quotes from Cache: {:?}", quotes);
    }
}
