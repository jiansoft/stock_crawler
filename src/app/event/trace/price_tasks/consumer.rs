//! 價格更新事件：發佈（去重後送進 channel）與 consumer（把代號交給追蹤 evaluator）。

use std::{collections::HashSet, sync::atomic::Ordering};

use rust_decimal::Decimal;
use tokio::{sync::mpsc, task};

use super::{
    PENDING_PRICE_SYMBOLS, PRICE_CONSUMER_ACTIVE_TASKS, PRICE_CONSUMER_LAST_GENERATION,
    PRICE_UPDATE_CHANNEL_CAPACITY, PRICE_UPDATE_TX, PriceUpdateEvent, stock_price, trace_stats,
};
use crate::core::util::atomic::decrement_atomic_usize;

/// 發佈單筆價格更新事件。
///
/// 事件只代表「這支股票的共享快取已更新」。
/// 實際追蹤比對時會重新從快取讀值，而不是直接使用這裡傳入的 `price`。
///
/// 若 trace 價格事件 consumer 尚未啟動或已停止，事件會直接被忽略。
pub fn publish_price_update(symbol: String, price: Decimal) {
    if price == Decimal::ZERO {
        return;
    }

    if !stock_price::has_targets_for_symbol(&symbol) {
        return;
    }

    let should_enqueue = match PENDING_PRICE_SYMBOLS.write() {
        Ok(mut pending) => pending.insert(symbol.clone()),
        Err(_) => {
            trace_stats::record_dropped_price_event();
            return;
        }
    };

    if !should_enqueue {
        return;
    }

    let tx = match PRICE_UPDATE_TX.read() {
        Ok(tx) => tx.as_ref().cloned(),
        Err(_) => None,
    };

    let send_result = tx.map(|tx| {
        tx.try_send(PriceUpdateEvent {
            symbol: symbol.clone(),
        })
        .is_ok()
    });

    if send_result == Some(true) {
        trace_stats::record_published_price_event();
        return;
    }

    clear_pending_price_symbol(&symbol);
    trace_stats::record_dropped_price_event();
}

/// 批次發佈多筆價格更新事件。
pub fn publish_price_updates(updates: Vec<(String, Decimal)>) {
    for (symbol, price) in updates {
        publish_price_update(symbol, price);
    }
}

/// 啟動價格更新事件 consumer。
pub(super) fn start_price_update_consumer_task() {
    let mut tx_guard = match PRICE_UPDATE_TX.write() {
        Ok(guard) => guard,
        Err(why) => {
            tracing::error!("Failed to lock price update sender because {:?}", why);
            return;
        }
    };

    if tx_guard.is_some() {
        return;
    }

    clear_pending_price_symbols();

    let (tx, mut rx) = mpsc::channel::<PriceUpdateEvent>(PRICE_UPDATE_CHANNEL_CAPACITY);
    *tx_guard = Some(tx);
    drop(tx_guard);
    let generation = PRICE_CONSUMER_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = PRICE_CONSUMER_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(
            "股票追蹤價格事件 consumer 啟動 generation={} active_tasks={}",
            generation,
            active_tasks
        );

        while let Some(event) = rx.recv().await {
            let symbol = event.symbol;
            trace_stats::record_consumed_price_event();
            // consumer 只通知「哪支股票剛更新」，
            // evaluator 會自行回頭讀共享快取來做高低標判斷。
            if let Err(why) = stock_price::evaluate_price_update(symbol.clone()).await {
                tracing::error!("Failed to evaluate price update event because {:?}", why);
            }
            clear_pending_price_symbol(&symbol);
        }

        clear_pending_price_symbols();
        let active_tasks = decrement_atomic_usize(&PRICE_CONSUMER_ACTIVE_TASKS);
        tracing::info!(
            "股票追蹤價格事件 consumer 已停止 generation={} active_tasks={}",
            generation,
            active_tasks
        );
    });
}

/// 停止價格更新事件 consumer。
pub(super) fn stop_price_update_consumer_task() {
    if let Ok(mut tx) = PRICE_UPDATE_TX.write() {
        tx.take();
    }
    clear_pending_price_symbols();
}

pub(super) fn clear_pending_price_symbol(symbol: &str) {
    if let Ok(mut pending) = PENDING_PRICE_SYMBOLS.write() {
        pending.remove(symbol);
    }
}

pub(super) fn clear_pending_price_symbols() {
    if let Ok(mut pending) = PENDING_PRICE_SYMBOLS.write() {
        *pending = HashSet::new();
    }
}

pub(super) fn pending_price_symbols_len() -> usize {
    PENDING_PRICE_SYMBOLS
        .read()
        .map(|pending| pending.len())
        .unwrap_or_default()
}
