use std::sync::atomic::Ordering;
use std::time::Duration;

use once_cell::sync::Lazy;
use scraper::{Html, Selector};
use tokio::sync::Mutex;

use super::{parse::*, screen::*, task::*, *};
use crate::core::util::diagnostics::ProcessMemoryStats;
use rust_decimal_macros::dec;

static TEST_STATE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// 驗證全量快取更新前，只會針對價格實際異動的股票產生價格事件。
#[test]
fn test_collect_changed_price_updates() {
    let _guard = TEST_STATE_LOCK.blocking_lock();
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
    assert_eq!(snapshot.source_site, "HiStock");
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
    assert_eq!(snapshot.source_site, "HiStock");
    assert_eq!(snapshot.change, Decimal::ZERO);
    assert_eq!(snapshot.volume, dec!(148));
}

#[test]
fn parse_row_skips_header_and_non_numeric_symbols() {
    let html = r#"<table>
        <tr><td>代號</td><td>名稱</td></tr>
        <tr><td>ABC</td><td>非股票列</td></tr>
    </table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");

    for row in fragment.select(&tr_selector) {
        assert!(parse_row(row).unwrap().is_none());
    }
}

#[test]
fn parse_row_keeps_positive_change_and_range() {
    let html = r#"<table><tr>
        <td>2330</td><td>台積電</td><td>1000</td>
        <td><span class="price-up">▲+15.00</span></td>
        <td><span class="price-up">+1.52%</span></td>
        <td>...</td><td>...</td><td>990</td><td>1005</td><td>985</td><td>985</td><td>1,234</td>
    </tr></table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
    let row = fragment.select(&tr_selector).next().unwrap();
    let (symbol, snapshot) = parse_row(row).unwrap().unwrap();

    assert_eq!(symbol, "2330");
    assert_eq!(snapshot.change, dec!(15));
    assert_eq!(snapshot.change_range, dec!(1.52));
    assert_eq!(snapshot.open, dec!(990));
    assert_eq!(snapshot.high, dec!(1005));
    assert_eq!(snapshot.low, dec!(985));
    assert_eq!(snapshot.last_close, dec!(985));
    assert_eq!(snapshot.volume, dec!(1234));
}

#[test]
fn parse_row_reports_malformed_numeric_fields() {
    let html = r#"<table><tr>
        <td>2330</td><td>台積電</td><td>not-a-price</td>
        <td>--</td><td>--</td><td>...</td><td>...</td>
        <td>990</td><td>1005</td><td>985</td><td>985</td><td>1234</td>
    </tr></table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
    let row = fragment.select(&tr_selector).next().unwrap();
    let error = parse_row(row).unwrap_err().to_string();

    assert!(error.contains("Failed to parse price for 2330"));
}

/// 以貼近真實排行榜頁面形狀的 fixture 驗證整頁解析流程。
///
/// 上面的 `test_parse_*` 系列只餵單一 `<tr>` 給 `parse_row`，
/// 這裡則走 [`parse_rank_html`] 完整路徑，額外驗證：
/// `#CPHB1_gv tr` 選擇器命中、表頭列略過、含英文字母代號
/// （債券 ETF）略過，以及 row_count / body_bytes 診斷數字正確。
#[test]
fn parse_rank_html_parses_fixture_rows() {
    // include_str! 的路徑相對於本檔案（histock/price/tests.rs）→ histock/testdata/。
    // 在「編譯期」把檔案內容嵌進測試執行檔，執行時不做任何檔案 I/O。
    const FIXTURE: &str = include_str!("../testdata/rank_page.html");

    let result = parse_rank_html(FIXTURE).unwrap();

    // fixture 有 5 個 <tr>：表頭 + 4 檔股票；全部都要被掃到（診斷用）。
    assert_eq!(result.row_count, 5);
    assert_eq!(result.body_bytes, FIXTURE.len());
    // 有效快照只有 3 檔：表頭列（代號非數字）與 00687B（含英文字母的
    // 債券 ETF 代號，依現行規則不納入）都被略過。
    assert_eq!(result.snapshots.len(), 3);
    assert!(!result.snapshots.contains_key("00687B"));

    // 下跌列：▼ 前綴 → 漲跌與幅都轉為負值。
    let down = &result.snapshots["5274"];
    assert_eq!(down.name, "信驊");
    assert_eq!(down.price, dec!(9445));
    assert_eq!(down.change, dec!(-55));
    assert_eq!(down.change_range, dec!(-0.58));

    // 上漲列：▲ 與 + 前綴維持正值；成交量的千分位逗號要被正確去除。
    let up = &result.snapshots["2330"];
    assert_eq!(up.change, dec!(15));
    assert_eq!(up.change_range, dec!(1.52));
    assert_eq!(up.volume, dec!(31415));
    assert_eq!(up.last_close, dec!(985));

    // 平盤列：「--」的漲跌與幅視為 0，不是解析錯誤。
    let flat = &result.snapshots["6584"];
    assert_eq!(flat.change, Decimal::ZERO);
    assert_eq!(flat.change_range, Decimal::ZERO);
}

/// 建立一列成交 78.5、漲 3.1、昨收 75.4、高 79、低 76 的正常快照。
fn consistent_snapshot(symbol: &str) -> RealtimeSnapshot {
    let mut snapshot = RealtimeSnapshot::new(symbol.to_string(), dec!(78.5));
    snapshot.change = dec!(3.1);
    snapshot.last_close = dec!(75.4);
    snapshot.high = dec!(79);
    snapshot.low = dec!(76);
    snapshot
}

/// 欄位彼此吻合的列不會被判為矛盾；尚未成交的列不檢查。
#[test]
fn row_inconsistency_accepts_consistent_and_untraded_rows() {
    assert_eq!(row_inconsistency(&consistent_snapshot("1301"), None), None);

    let mut untraded = consistent_snapshot("1301");
    untraded.price = Decimal::ZERO;
    assert_eq!(row_inconsistency(&untraded, None), None);

    // 平盤列：漲跌「--」解析為 0，成交價等於昨收。
    let mut flat = consistent_snapshot("6584");
    flat.price = dec!(75.4);
    flat.change = Decimal::ZERO;
    flat.low = dec!(75);
    assert_eq!(row_inconsistency(&flat, None), None);

    // 沒有最高最低與昨收（例如剛開盤欄位還是「--」）時無從比對，視為吻合。
    let mut sparse = RealtimeSnapshot::new("2486".to_string(), dec!(236));
    sparse.change = dec!(-4);
    assert_eq!(row_inconsistency(&sparse, None), None);
}

/// 10-08 開盤實例：1301 成交欄變成 33，但昨收與最高最低仍是自己的。
#[test]
fn row_inconsistency_rejects_price_outside_high_low() {
    let mut snapshot = consistent_snapshot("1301");
    snapshot.price = dec!(33);
    assert_eq!(
        row_inconsistency(&snapshot, None),
        Some("成交價不在最高最低之間")
    );
}

/// 成交價在高低之間但與漲跌對不上（錯在漲跌停範圍內）也要抓出來。
#[test]
fn row_inconsistency_rejects_price_that_disagrees_with_change() {
    let mut snapshot = consistent_snapshot("1301");
    snapshot.price = dec!(77);
    assert_eq!(
        row_inconsistency(&snapshot, None),
        Some("成交價減漲跌不等於昨收")
    );
}

/// 除權息當天漲跌以參考價為準：成交價減漲跌等於參考價也算吻合。
#[test]
fn row_inconsistency_accepts_change_against_reference_price() {
    let mut snapshot = consistent_snapshot("2886");
    snapshot.price = dec!(40);
    snapshot.change = dec!(0.5);
    snapshot.last_close = dec!(41);
    snapshot.high = dec!(40.5);
    snapshot.low = dec!(39.2);

    assert_eq!(
        row_inconsistency(&snapshot, None),
        Some("成交價減漲跌不等於昨收")
    );
    assert_eq!(row_inconsistency(&snapshot, Some(dec!(39.5))), None);
}

fn fetch_result_with(snapshots: Vec<RealtimeSnapshot>) -> HiStockFetchResult {
    let row_count = snapshots.len();
    HiStockFetchResult {
        snapshots: snapshots
            .into_iter()
            .map(|snapshot| (snapshot.symbol.clone(), snapshot))
            .collect(),
        body_bytes: 0,
        row_count,
    }
}

/// 少數矛盾列只把該列價格歸零（交給快取保留舊值），其餘照常使用。
#[test]
fn screen_inconsistent_rows_zeroes_only_bad_rows() {
    let mut bad = consistent_snapshot("79971");
    bad.price = dec!(33);
    let mut result = fetch_result_with(vec![
        consistent_snapshot("79970"),
        bad,
        consistent_snapshot("79972"),
    ]);

    let removed = screen_inconsistent_rows(&mut result, |_| None).unwrap();

    assert_eq!(removed, 1);
    assert_eq!(result.snapshots.len(), 3);
    assert_eq!(result.snapshots["79971"].price, Decimal::ZERO);
    assert_eq!(result.snapshots["79970"].price, dec!(78.5));
    assert_eq!(result.snapshots["79972"].price, dec!(78.5));
}

/// 矛盾列超過 max(20, 2%) 代表整頁錯亂，整批捨棄。
#[test]
fn screen_inconsistent_rows_rejects_dirty_batch() {
    let snapshots = (0..30)
        .map(|i| {
            let mut snapshot = consistent_snapshot(&format!("7990{i:02}"));
            if i < 21 {
                snapshot.price = dec!(33);
            }
            snapshot
        })
        .collect();
    let mut result = fetch_result_with(snapshots);

    let error = screen_inconsistent_rows(&mut result, |_| None).unwrap_err();

    let dirty = error.downcast_ref::<DirtyBatch>().expect("DirtyBatch 錯誤");
    assert_eq!(dirty.inconsistent, 21);
    assert_eq!(dirty.total, 30);
    assert!(error.to_string().contains("21/30"));
}

/// 剛好 20 列矛盾還在容許範圍內，只剔除那幾列。
#[test]
fn screen_inconsistent_rows_tolerates_up_to_limit() {
    let snapshots = (0..30)
        .map(|i| {
            let mut snapshot = consistent_snapshot(&format!("7990{i:02}"));
            if i < 20 {
                snapshot.price = dec!(33);
            }
            snapshot
        })
        .collect();
    let mut result = fetch_result_with(snapshots);

    assert_eq!(screen_inconsistent_rows(&mut result, |_| None).unwrap(), 20);
}

/// 真實頁面形狀的 fixture 不應有任何列被判為矛盾。
#[test]
fn screen_inconsistent_rows_keeps_fixture_page_intact() {
    const FIXTURE: &str = include_str!("../testdata/rank_page.html");
    let mut result = parse_rank_html(FIXTURE).unwrap();

    assert_eq!(screen_inconsistent_rows(&mut result, |_| None).unwrap(), 0);
    assert!(result.snapshots.values().all(|s| s.price > Decimal::ZERO));
}

/// 整頁完全沒有可用列（例如 HiStock 改版換了表格 id）時必須報錯，
/// 讓上層知道快取「這一輪沒有更新」，而不是默默拿空資料覆蓋。
#[test]
fn parse_rank_html_rejects_page_without_rank_table() {
    let error = parse_rank_html("<html><body><p>維護中</p></body></html>").unwrap_err();
    assert!(error.to_string().contains("empty map"));
}

#[test]
fn diagnostics_snapshot_reflects_idle_state_and_runtime_counters() {
    IS_CACHING.store(false, Ordering::SeqCst);
    ACTIVE_TASKS.store(0, Ordering::SeqCst);
    LAST_BODY_BYTES.store(1024, Ordering::SeqCst);
    LAST_ROW_COUNT.store(30, Ordering::SeqCst);
    LAST_SNAPSHOT_COUNT.store(28, Ordering::SeqCst);
    LAST_CHANGED_EVENT_COUNT.store(4, Ordering::SeqCst);
    LAST_ELAPSED_MS.store(250, Ordering::SeqCst);
    LAST_RSS_DELTA_KIB.store(-12, Ordering::SeqCst);
    COMPLETED_CYCLES.store(3, Ordering::SeqCst);

    let diagnostics = runtime_diagnostics_snapshot();

    assert!(!diagnostics.status.enabled);
    assert_eq!(diagnostics.status.active_tasks, 0);
    assert_eq!(diagnostics.last_body_bytes, 1024);
    assert_eq!(diagnostics.last_row_count, 30);
    assert_eq!(diagnostics.last_snapshot_count, 28);
    assert_eq!(diagnostics.last_changed_event_count, 4);
    assert_eq!(diagnostics.last_elapsed_ms, 250);
    assert_eq!(diagnostics.last_rss_delta_kib, -12);
    assert_eq!(diagnostics.completed_cycles, 3);
}

#[tokio::test]
async fn test_cache_mechanism() {
    let _guard = TEST_STATE_LOCK.lock().await;
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
    let _guard = TEST_STATE_LOCK.lock().await;
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
    let _guard = TEST_STATE_LOCK.lock().await;
    dotenvy::dotenv().ok();
    tracing::debug!("開始 HiStock::get_stock_price");

    match HiStock::get_stock_price("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("HiStock::get_stock_price : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_get_stock_quotes() {
    let _guard = TEST_STATE_LOCK.lock().await;
    dotenvy::dotenv().ok();
    tracing::debug!("開始 HiStock::get_stock_quotes");

    match HiStock::get_stock_quotes("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("HiStock::get_stock_quotes : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to HiStock::get_stock_quotes because {:?}", why);
        }
    }
}
#[tokio::test]
#[ignore]
async fn test_get_stock_price_with_cache_verification() {
    let _guard = TEST_STATE_LOCK.lock().await;
    // 1. 清空快取
    stop_caching_task().await;

    // 2. 抓取全量並填充快取
    let all_stocks = fetch_all_from_rank().await.unwrap().snapshots;
    SHARE.set_stock_snapshots(all_stocks);

    // 3. 測試透過公用介面取得價格 (此時應從快取秒讀)
    let price = HiStock::get_stock_price("2330").await.unwrap();
    assert!(price > Decimal::ZERO);
    println!("Verified Price from Cache: {}", price);
}

#[tokio::test]
#[ignore]
async fn test_get_stock_quotes_with_cache_verification() {
    let _guard = TEST_STATE_LOCK.lock().await;
    // 1. 清空快取
    stop_caching_task().await;

    // 2. 抓取全量並填充快取
    let all_stocks = fetch_all_from_rank().await.unwrap().snapshots;
    SHARE.set_stock_snapshots(all_stocks);

    // 3. 測試透過公用介面取得報價物件
    let quotes = HiStock::get_stock_quotes("2330").await.unwrap();
    assert_eq!(quotes.stock_symbol, "2330");
    assert!(quotes.price > 0.0);
    println!("Verified Quotes from Cache: {:?}", quotes);
}

/// RSS 差值：前後都讀得到才相減（可為負），任一側讀不到就回 0。
#[test]
fn rss_delta_kib_needs_both_samples() {
    let sample = |vm_rss_kib| ProcessMemoryStats {
        vm_rss_kib,
        vm_size_kib: 0,
    };

    assert_eq!(rss_delta_kib(Some(sample(1_000)), Some(sample(1_250))), 250);
    assert_eq!(
        rss_delta_kib(Some(sample(1_250)), Some(sample(1_000))),
        -250
    );
    assert_eq!(rss_delta_kib(None, Some(sample(1_000))), 0);
    assert_eq!(rss_delta_kib(Some(sample(1_000)), None), 0);
}

/// 快取命中時，`StockInfo` 兩個介面直接從快取取值，不會觸發全量重抓。
#[tokio::test]
async fn stock_info_reads_from_cache_when_hit() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;

    let mut snapshot = consistent_snapshot("79974");
    snapshot.change_range = dec!(4.11);
    {
        let mut cache = SHARE.stock_snapshots.write().unwrap();
        cache.insert("79974".to_string(), snapshot);
    }

    let price = HiStock::get_stock_price("79974").await.unwrap();
    let quotes = HiStock::get_stock_quotes("79974").await.unwrap();

    assert_eq!(price, dec!(78.5));
    assert_eq!(quotes.stock_symbol, "79974");
    assert_eq!(quotes.price, 78.5);
    assert_eq!(quotes.change, 3.1);
    assert_eq!(quotes.change_range, 4.11);

    stop_caching_task().await;
}

/// 同一檔當天第二次前後矛盾只記 debug，但仍照樣剔除該列。
#[test]
fn screen_inconsistent_rows_zeroes_repeated_bad_row() {
    for _ in 0..2 {
        let mut bad = consistent_snapshot("79975");
        bad.price = dec!(33);
        let mut result = fetch_result_with(vec![bad, consistent_snapshot("79976")]);

        let removed = screen_inconsistent_rows(&mut result, |_| None).unwrap();

        assert_eq!(removed, 1);
        assert_eq!(result.snapshots["79975"].price, Decimal::ZERO);
        assert_eq!(result.snapshots["79976"].price, dec!(78.5));
    }
}
