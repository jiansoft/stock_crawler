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
//! 6. **頻率限制**：於設定的時間窗（預設 1 小時）內，對「同一股票、同一邊界方向」
//!    只在報價創新低（floor）或新高（ceiling）時才發送警報，避免同方向、未創極端的報價持續洗版。
//!    時間窗以記憶體 TTL 與 Redis 雙層保存「已通知過的極端值」基準。
//! 7. **發送通知**：透過 Telegram Bot 將警報訊息傳送給使用者。
//!
//! ## 子模組
//! - [`reference`]：開盤前載入當日除權息、減資恢復買賣的參考價。
//! - [`targets`]：依股票代號分組的追蹤條件快取。
//! - [`alert`]：邊界判斷、去重與警報訊息。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use futures::future;
use rust_decimal::Decimal;
use tokio::{task, time};

use super::price_tasks as trace_price_tasks;
use crate::{
    core::declare,
    core::util::datetime::Weekend,
    domain::trace::entity::PriceTrace,
    infra::cache::{RealtimeSnapshot, SHARE},
    infra::crawler::twse,
};

/// 邊界判斷、去重與警報訊息。
mod alert;
/// 當日除權息、減資恢復買賣參考價。
mod reference;
/// 追蹤條件快取。
mod targets;

use alert::alert_on_price_boundary;
use reference::load_reference_prices;
pub(super) use targets::{
    clear_trace_targets_cache, get_tracked_symbols, has_loaded_trace_targets_cache,
    has_targets_for_symbol, refresh_trace_targets_cache, trace_target_diagnostics,
};
use targets::{get_grouped_targets_snapshot, get_targets_by_symbol};

/// 確保整個追蹤執行流程只有一個實例在執行。
static IS_RUNNING: AtomicBool = AtomicBool::new(false);

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
/// [`SHARE`](SHARE) 中的 `stock_snapshots` 快取，再由價格更新事件
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
        tracing::debug!("股票追蹤任務已在運行中，跳過重複啟動");
        return Ok(());
    }

    // 先載入當日除權息與減資恢復買賣參考價，讓開盤第一批報價就用正確的基準檢查異常價格。
    // 失敗只影響這些股票的過濾準確度，不阻擋追蹤啟動。
    load_reference_prices(now.date_naive()).await;

    // 啟動背景監控任務
    task::spawn(async move {
        // 先啟動 trace 層的即時報價背景任務與價格事件 consumer。
        if let Err(why) = trace_price_tasks::start_price_tasks().await {
            tracing::error!("Failed to start trace price tasks because {:?}", why);
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
            tracing::debug!("已達關盤時間，停止追蹤任務");
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
            tracing::info!(
                "Today is a holiday ({}), and the market is closed.",
                holiday.why
            );
            return Ok(true);
        }
    }

    Ok(false)
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
                EvaluationSource::Reconciliation,
            ))
        })
        .collect::<Vec<_>>();

    future::join_all(futures).await;
    Ok(symbol_count)
}

/// 以價格更新事件驅動指定股票的追蹤條件檢查。
///
/// 此入口只把「哪支股票剛更新」交給 evaluator，
/// 實際拿來比對高低標的價格會重新從共享快取讀取。
///
/// # 參數
/// - `symbol`: 發生價格更新的股票代號。
pub(super) async fn evaluate_price_update(symbol: String) -> Result<()> {
    let targets = get_targets_by_symbol(&symbol);
    if targets.is_empty() {
        return Ok(());
    }

    process_cached_targets(symbol, targets, EvaluationSource::PriceEvent).await;
    Ok(())
}

/// 從即時報價快取讀取指定股票的最新成交價。
///
/// # 回傳
/// - `Some(snapshot)`：快取內已有該股票最新成交價與來源資訊。
/// - `None`：快取尚未暖身完成，或該股票目前不存在於快取中。
fn get_cached_snapshot(symbol: &str) -> Option<RealtimeSnapshot> {
    SHARE.get_stock_snapshot(symbol)
}

/// 處理同一支股票的多個追蹤目標。
///
/// 1. 統一從即時報價快取讀取目前價格。
/// 2. 若價格有效（非零），則檢查該股票的所有追蹤目標是否觸發警報。
///
/// 不論觸發來源是價格事件還是低頻 reconciliation，
/// 這裡都統一從共享快取取值，避免不同路徑使用不同價格來源。
async fn process_cached_targets(
    symbol: String,
    targets: Vec<PriceTrace>,
    source: EvaluationSource,
) {
    let snapshot = get_cached_snapshot(&symbol);

    match snapshot {
        Some(snapshot) if snapshot.price != Decimal::ZERO => {
            let current_price = snapshot.price;
            let source_site =
                (!snapshot.source_site.trim().is_empty()).then_some(snapshot.source_site.as_str());
            for target in targets {
                if let Err(why) =
                    alert_on_price_boundary(target, current_price, source, source_site).await
                {
                    tracing::error!("Error alerting for {}: {:?}", symbol, why);
                }
            }
        }
        Some(_) => {
            // 盤中每檔股票、每次價格事件都可能觸發，屬高頻雜訊，降為 trace 避免日誌暴增。
            tracing::trace!("Stock {} current price is zero, skipping", symbol);
        }
        None => {
            tracing::trace!("Stock {} snapshot cache miss, skipping", symbol);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rust_decimal_macros::dec;

    use super::*;

    /// 驗證即時報價快取讀取可正確命中與 miss。
    #[test]
    fn test_get_cached_snapshot() {
        // 確保全域 SHARE 的昨收快取與報價快取都是乾淨的，
        // 避免其他測試在 SHARE.last_trading_day_quotes 留下真實收盤價，
        // 導致 is_valid_price 判斷 dec!(998) 偏差過大而拒絕此快照。
        SHARE.clear_last_trading_day_quotes();
        SHARE.clear_stock_snapshots();

        let mut snapshots = HashMap::new();
        let mut snapshot = RealtimeSnapshot::new("2330".to_string(), dec!(998));
        snapshot.source_site = "Yahoo".to_string();
        snapshots.insert("2330".to_string(), snapshot);
        SHARE.set_stock_snapshots(snapshots);

        let snapshot = get_cached_snapshot("2330").unwrap();
        assert_eq!(snapshot.price, dec!(998));
        assert_eq!(snapshot.source_site, "Yahoo");
        assert!(get_cached_snapshot("2317").is_none());

        SHARE.clear_stock_snapshots();
    }

    #[tokio::test]
    #[ignore]
    async fn test_reconcile_target_prices() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        refresh_trace_targets_cache().await.unwrap();
        let result = reconcile_target_prices().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        let result = execute().await;
        assert!(result.is_ok());
    }
}
