//! # 股票價格追蹤與提醒模組
//!
//! 此模組負責監控使用者設定的追蹤股票（Trace），並在股價超過預設的高低標時發送通知。
//!
//! ## 主要流程
//! 1. **檢查開盤狀態**：判斷當前是否為交易日（非週末且非假日）。
//! 2. **啟動即時報價背景採集**：透過 trace 協調層啟動全市場採集、備援採集、價格事件 consumer 與追蹤條件快取刷新任務。
//! 3. **價格更新事件驅動判斷**：當背景採集更新股價後，會主動觸發指定股票的追蹤條件檢查。
//! 4. **低頻對帳掃描**：保留低頻 reconciliation 任務，補償事件遺漏、設定剛新增但價格尚未再次變動等情況。
//! 5. **邊界檢查**：判斷最新價格是否低於設定的最低價（Floor）或超過最高價（Ceiling）。
//! 6. **頻率限制**：利用 Redis 記錄已發送過的提醒，避免在短時間內重複發送相同的警報。
//! 7. **發送通知**：透過 Telegram Bot 將警報訊息傳送給使用者。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Duration;

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use futures::future;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::{task, time};

use super::{price_tasks as trace_price_tasks, stats as trace_stats};
use crate::bot::telegram::Telegram;
use crate::{
    bot,
    cache::SHARE,
    crawler::twse,
    database::table::trace::Trace,
    declare, logging, nosql,
    util::{datetime::Weekend, map::Keyable},
};

/// 確保整個追蹤執行流程只有一個實例在執行。
static IS_RUNNING: AtomicBool = AtomicBool::new(false);
/// 依股票代號分組後的追蹤條件快取。
static TRACE_TARGETS: Lazy<RwLock<HashMap<String, Vec<Trace>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
/// 標記追蹤條件快取是否至少成功載入過一次。
static TRACE_TARGETS_LOADED: AtomicBool = AtomicBool::new(false);

/// 追蹤條件判斷的觸發來源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvaluationSource {
    /// 由價格更新事件直接觸發。
    PriceEvent,
    /// 由低頻 reconciliation 補償掃描觸發。
    Reconciliation,
}

/// 執行股票價格追蹤任務的入口點。
///
/// 此函式會先進行基本的檢查（是否為週末或假日），如果符合追蹤條件，
/// 則會啟動一個非同步任務來完成三件事：
/// 1. 啟動 trace 層的即時報價背景任務與價格事件 consumer。
/// 2. 在快取暖身完成後，維持開盤期間的追蹤生命週期。
/// 3. 追蹤結束時停止所有 trace 相關背景任務。
///
/// 追蹤任務本身不直接對外網站採集報價，而是由背景採集器寫入
/// [`SHARE`](crate::cache::SHARE) 中的 `stock_snapshots` 快取，再由價格更新事件
/// 驅動 [`evaluate_price_update`] 執行指定股票的邊界檢查。
///
/// # Errors
///
/// 如果在檢查假期時發生資料庫或網路錯誤，將會回傳 `Err`。
pub async fn execute() -> Result<()> {
    let now = Local::now();

    // 週末不處理
    if now.is_weekend() {
        return Ok(());
    }

    // 檢查是否為國定假日休市
    if is_holiday(now.date_naive()).await? {
        return Ok(());
    }

    // 檢查是否已經在運行，避免重複啟動
    if IS_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        logging::debug_file_async("股票追蹤任務已在運行中，跳過重複啟動".to_string());
        return Ok(());
    }

    // 啟動背景監控任務
    task::spawn(async move {
        // 先啟動 trace 層的即時報價背景任務與價格事件 consumer。
        if let Err(why) = trace_price_tasks::start_price_tasks().await {
            logging::error_file_async(format!(
                "Failed to start trace price tasks because {:?}",
                why
            ));
            IS_RUNNING.store(false, Ordering::SeqCst);
            return;
        }

        // 等待共用快取至少先暖身一次，降低開盤初期全數 cache miss 的機率。
        trace_price_tasks::wait_for_price_cache_ready().await;

        // 僅維持追蹤任務的生命週期，實際警報判斷改由價格更新事件驅動。
        wait_until_market_close().await;

        // 關盤後停止 trace 層的即時報價背景任務。
        trace_price_tasks::stop_price_tasks().await;
        // 釋放執行中旗標，讓下一輪排程可以重新啟動追蹤任務。
        IS_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// 等待台股市場進入關盤狀態。
///
/// 此任務不再負責定期掃描追蹤條件，而是只維持追蹤任務的生命週期；
/// 一旦偵測到 [`declare::StockExchange::TWSE`] 已關盤，便結束流程並交由上層收尾。
///
/// 這裡每 5 秒檢查一次關盤狀態，讓背景任務能在收盤後較快停止，
/// 同時避免每秒輪詢帶來不必要的喚醒成本。
async fn wait_until_market_close() {
    let mut ticker = time::interval(Duration::from_secs(5));

    loop {
        if !declare::StockExchange::TWSE.is_open() {
            logging::debug_file_async("已達關盤時間，停止追蹤任務".to_string());
            break;
        }

        ticker.tick().await;
    }
}

/// 判斷特定日期是否為台灣證券交易所（TWSE）公告的休假日。
async fn is_holiday(today: NaiveDate) -> Result<bool> {
    let holidays = match twse::holiday_schedule::visit(today.year()).await {
        Ok(result) => result,
        Err(err) => {
            anyhow::bail!("Failed to visit TWSE holiday schedule: {:?}", err);
        }
    };

    for holiday in holidays {
        if holiday.date == today {
            logging::info_file_async(format!(
                "Today is a holiday ({}), and the market is closed.",
                holiday.why
            ));
            return Ok(true);
        }
    }

    Ok(false)
}

/// 以股票代號將追蹤條件分組。
fn group_targets_by_symbol(targets: Vec<Trace>) -> HashMap<String, Vec<Trace>> {
    let mut grouped_targets = HashMap::new();
    for target in targets {
        grouped_targets
            .entry(target.stock_symbol.clone())
            .or_insert_with(Vec::new)
            .push(target);
    }

    grouped_targets
}

/// 重新整理追蹤條件快取。
///
/// 此快取會依股票代號分組，供價格更新事件與低頻 reconciliation 共用，
/// 避免在每次價格變動時都重新查詢整張 `trace` 資料表。
pub(super) async fn refresh_trace_targets_cache() -> Result<usize> {
    let targets = Trace::fetch().await?;
    let grouped_targets = group_targets_by_symbol(targets);
    let symbol_count = grouped_targets.len();

    if let Ok(mut cache) = TRACE_TARGETS.write() {
        *cache = grouped_targets;
    }

    TRACE_TARGETS_LOADED.store(true, Ordering::SeqCst);
    Ok(symbol_count)
}

/// 判斷追蹤條件快取是否至少成功載入過一次。
pub(super) fn has_loaded_trace_targets_cache() -> bool {
    TRACE_TARGETS_LOADED.load(Ordering::SeqCst)
}

/// 取得追蹤條件快取的快照。
fn get_grouped_targets_snapshot() -> HashMap<String, Vec<Trace>> {
    TRACE_TARGETS
        .read()
        .map(|cache| cache.clone())
        .unwrap_or_default()
}

/// 取得指定股票代號的追蹤條件清單。
fn get_targets_by_symbol(symbol: &str) -> Vec<Trace> {
    TRACE_TARGETS
        .read()
        .ok()
        .and_then(|cache| cache.get(symbol).cloned())
        .unwrap_or_default()
}

/// 取得目前追蹤條件快取中的股票代號清單。
pub(super) fn get_tracked_symbols() -> Vec<String> {
    let mut symbols = TRACE_TARGETS
        .read()
        .map(|cache| cache.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    symbols.sort();
    symbols
}

/// 低頻對帳掃描目前追蹤中的股票。
///
/// 此方法會從記憶體中的追蹤條件快取出發，對每個股票重新讀取目前快取價格，
/// 作為價格事件遺漏、程式重啟或新追蹤條件尚未等到下一次價格變動時的補償機制。
pub(super) async fn reconcile_target_prices() -> Result<usize> {
    let grouped_targets = get_grouped_targets_snapshot();
    let symbol_count = grouped_targets.len();
    if grouped_targets.is_empty() {
        return Ok(0);
    }

    let futures = grouped_targets
        .into_iter()
        .map(|(symbol, targets)| {
            task::spawn(process_cached_targets(
                symbol,
                targets,
                None,
                EvaluationSource::Reconciliation,
            ))
        })
        .collect::<Vec<_>>();

    future::join_all(futures).await;
    Ok(symbol_count)
}

/// 以價格更新事件驅動指定股票的追蹤條件檢查。
///
/// # 參數
/// - `symbol`: 發生價格更新的股票代號。
/// - `current_price`: 最新成交價。
pub(super) async fn evaluate_price_update(symbol: String, current_price: Decimal) -> Result<()> {
    let targets = get_targets_by_symbol(&symbol);
    if targets.is_empty() {
        return Ok(());
    }

    process_cached_targets(
        symbol,
        targets,
        Some(current_price),
        EvaluationSource::PriceEvent,
    )
    .await;
    Ok(())
}

/// 從即時報價快取讀取指定股票的最新成交價。
///
/// # 回傳
/// - `Some(price)`：快取內已有該股票最新成交價。
/// - `None`：快取尚未暖身完成，或該股票目前不存在於快取中。
fn get_cached_current_price(symbol: &str) -> Option<Decimal> {
    SHARE
        .get_stock_snapshot(symbol)
        .map(|snapshot| snapshot.price)
}

/// 處理同一支股票的多個追蹤目標。
///
/// 1. 若有事件帶入 `event_price`，優先使用該價格做判斷。
/// 2. 否則從即時報價快取讀取目前價格，供 reconciliation 使用。
/// 3. 若價格有效（非零），則檢查該股票的所有追蹤目標是否觸發警報。
async fn process_cached_targets(
    symbol: String,
    targets: Vec<Trace>,
    event_price: Option<Decimal>,
    source: EvaluationSource,
) {
    let current_price = event_price.or_else(|| get_cached_current_price(&symbol));

    match current_price {
        Some(current_price) if current_price != Decimal::ZERO => {
            for target in targets {
                if let Err(why) = alert_on_price_boundary(target, current_price, source).await {
                    logging::error_file_async(format!("Error alerting for {}: {:?}", symbol, why));
                }
            }
        }
        Some(_) => {
            logging::debug_file_async(format!("Stock {} current price is zero, skipping", symbol));
        }
        None => {
            logging::debug_file_async(format!("Stock {} snapshot cache miss, skipping", symbol));
        }
    }
}

/// 判斷股價是否觸發警報，並在必要時發送通知。
///
/// 最佳化：快取 Key 改為 `symbol:boundary_type`，避免價位變動時每分鐘重複警報。
async fn alert_on_price_boundary(
    target: Trace,
    current_price: Decimal,
    source: EvaluationSource,
) -> Result<bool> {
    // 判斷當前價格是否在預定範圍內（如果在範圍內則不需提醒）
    if is_within_boundary(&target, current_price) {
        return Ok(false);
    }

    // 判定是觸發高標還是低標
    let boundary_type = if current_price < target.floor && target.floor > Decimal::ZERO {
        "floor"
    } else if current_price > target.ceiling && target.ceiling > Decimal::ZERO {
        "ceiling"
    } else {
        // 理論上不會走到這裡，因為 above implies !is_within_boundary
        return Ok(false);
    };

    // 檢查 Redis 快取，避免針對同一方向重複通知
    // Key 格式包含股票代號與邊界類型，存活時間設為 1 小時，避免頻繁轟炸
    let target_key = format!("{}:{}", target.key_with_prefix(), boundary_type);
    if let Ok(exist) = nosql::redis::CLIENT.contains_key(&target_key).await {
        if exist {
            return Ok(false);
        }
    }

    // 格式化訊息並發送
    let to_bot_msg = format_alert_message(&target, current_price).await;

    // 寫入快取 (有效期限 1 小時)
    if let Err(why) = nosql::redis::CLIENT
        .set(&target_key, current_price.to_string(), 60 * 60)
        .await
    {
        logging::error_file_async(format!("Failed to set Redis key {}: {:?}", target_key, why));
    }

    // 發送 Telegram 訊息
    bot::telegram::send(&to_bot_msg).await;
    trace_stats::record_notification_sent();
    if source == EvaluationSource::Reconciliation {
        trace_stats::record_reconciliation_alert_hit();
    }

    Ok(true)
}

/// 格式化警報訊息內容。
async fn format_alert_message(target: &Trace, current_price: Decimal) -> String {
    let stock_name = SHARE
        .get_stock(&target.stock_symbol)
        .await
        .map_or_else(String::new, |stock| stock.name);

    let (boundary, limit) = if current_price < target.floor && target.floor > Decimal::ZERO {
        ("低於最低價", target.floor)
    } else {
        ("超過最高價", target.ceiling)
    };

    let escaped_name = Telegram::escape_markdown_v2(stock_name);
    let escaped_boundary = Telegram::escape_markdown_v2(boundary.to_string());
    let escaped_limit = Telegram::escape_markdown_v2(limit.to_string());
    let escaped_price = Telegram::escape_markdown_v2(current_price.to_string());
    let symbol = &target.stock_symbol;

    format!("{escaped_name} {escaped_boundary}:{escaped_limit}，目前報價:{escaped_price} [Yahoo 股市](https://tw\\.stock\\.yahoo\\.com/quote/{symbol})")
}

/// 判斷當前價格是否在預定的 [floor, ceiling] 範圍內。
///
/// 如果設定值為 0，表示不限制該方向的邊界。
fn is_within_boundary(target: &Trace, current_price: Decimal) -> bool {
    let floor = target.floor;
    let ceiling = target.ceiling;

    match (floor > Decimal::ZERO, ceiling > Decimal::ZERO) {
        (true, true) => current_price >= floor && current_price <= ceiling,
        (true, false) => current_price >= floor,
        (false, true) => current_price <= ceiling,
        _ => true, // 如果都沒設定，視為在範圍內
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rust_decimal_macros::dec;

    use crate::cache::RealtimeSnapshot;

    use super::*;

    /// 驗證追蹤條件會依股票代號正確分組。
    #[test]
    fn test_group_targets_by_symbol() {
        let grouped = group_targets_by_symbol(vec![
            Trace::new("2330".to_string(), dec!(500), dec!(600)),
            Trace::new("2317".to_string(), dec!(100), dec!(120)),
            Trace::new("2330".to_string(), dec!(520), dec!(650)),
        ]);

        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped.get("2330").map(Vec::len), Some(2));
        assert_eq!(grouped.get("2317").map(Vec::len), Some(1));
    }

    /// 驗證價格區間判斷邏輯可正確處理雙邊界、單邊界與未設定情況。
    #[test]
    fn test_is_within_boundary() {
        // 設定高低標 (500 ~ 600)
        let mut trace = Trace {
            stock_symbol: "2330".to_string(),
            floor: dec!(500),
            ceiling: dec!(600),
        };

        // 邊界測試
        assert!(is_within_boundary(&trace, dec!(550)));
        assert!(is_within_boundary(&trace, dec!(500)));
        assert!(is_within_boundary(&trace, dec!(600)));
        assert!(!is_within_boundary(&trace, dec!(499.9)));
        assert!(!is_within_boundary(&trace, dec!(600.1)));

        // 僅設定低標 (>= 500)
        trace.ceiling = Decimal::ZERO;
        assert!(is_within_boundary(&trace, dec!(500)));
        assert!(is_within_boundary(&trace, dec!(1000)));
        assert!(!is_within_boundary(&trace, dec!(499.9)));

        // 僅設定高標 (<= 600)
        trace.floor = Decimal::ZERO;
        trace.ceiling = dec!(600);
        assert!(is_within_boundary(&trace, dec!(600)));
        assert!(is_within_boundary(&trace, dec!(0.1)));
        assert!(!is_within_boundary(&trace, dec!(600.1)));

        // 皆未設定
        trace.ceiling = Decimal::ZERO;
        assert!(is_within_boundary(&trace, dec!(123)));
    }

    /// 驗證即時報價快取讀取可正確命中與 miss。
    #[test]
    fn test_get_cached_current_price() {
        let mut snapshots = HashMap::new();
        snapshots.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(998)),
        );
        SHARE.set_stock_snapshots(snapshots);

        assert_eq!(get_cached_current_price("2330"), Some(dec!(998)));
        assert_eq!(get_cached_current_price("2317"), None);

        SHARE.clear_stock_snapshots();
    }

    #[tokio::test]
    #[ignore]
    async fn test_format_alert_message() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let trace = Trace {
            stock_symbol: "2330".to_string(),
            floor: dec!(500),
            ceiling: dec!(600),
        };

        // 觸發高標
        let msg = format_alert_message(&trace, dec!(650)).await;
        assert!(msg.contains("超過最高價"));
        assert!(msg.contains("目前報價:650"));

        // 觸發低標
        let msg = msg_low(&trace, dec!(450)).await;
        assert!(msg.contains("低於最低價"));
        assert!(msg.contains("目前報價:450"));
    }

    async fn msg_low(target: &Trace, price: Decimal) -> String {
        format_alert_message(target, price).await
    }

    #[tokio::test]
    #[ignore]
    async fn test_handle_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let trace = Trace {
            stock_symbol: "1303".to_string(),
            floor: dec!(70),
            ceiling: dec!(60),
        };

        let result = alert_on_price_boundary(trace, dec!(560), EvaluationSource::PriceEvent).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn test_reconcile_target_prices() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        refresh_trace_targets_cache().await.unwrap();
        let result = reconcile_target_prices().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let result = execute().await;
        assert!(result.is_ok());
    }
}
