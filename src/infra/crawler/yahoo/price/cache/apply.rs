//! 把單一類股的最新快照寫回共用即時快取，並對價格異動的股票發佈 trace 價格事件。

use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;

use super::super::class_quote;
use crate::{
    app::event::trace::price_tasks as trace_price_tasks,
    infra::cache::{RealtimeSnapshot, SHARE, report_abnormal_price},
    infra::crawler::yahoo::{YahooClassCategory, YahooClassExchange},
};

/// 單一類股快取套用後的摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ApplyCategoryResult {
    pub(super) total_snapshot_count: usize,
    pub(super) changed_event_count: usize,
}

/// 將單一類股的最新快照套用到共用快取。
///
/// 這個步驟會：
/// - 移除該類股上一輪存在、這一輪已消失的股票。
/// - 寫入該類股本輪抓到的最新快照。
/// - 回傳更新後整體快取的股票數量。
pub(super) fn apply_category_snapshots(
    category: &YahooClassCategory,
    category_snapshots: HashMap<String, RealtimeSnapshot>,
    category_symbols: &mut HashMap<String, HashSet<String>>,
) -> ApplyCategoryResult {
    // 先算出這個類股本輪的內部鍵值，讓同一個 sector 的 symbol 集可以被覆蓋更新。
    let category_key = class_quote::category_key(category);
    // 把本輪所有 symbol 收成集合，後面可以和上一輪做集合差異比對。
    let new_symbols: HashSet<String> = category_snapshots.keys().cloned().collect();
    // `insert` 會回傳舊集合；這正好拿來得知上一輪這個類股有哪些股票。
    let previous_symbols = category_symbols
        .insert(category_key, new_symbols)
        .unwrap_or_default();

    // 驗證前先記下本輪的漲跌停價：Yahoo 類股報價自帶當日漲跌幅限制，比固定的 10.5% 門檻準確，
    // 也讓其他備援站點的報價能用同一份限制驗證。
    SHARE.set_price_limits(
        chrono::Local::now().date_naive(),
        category_snapshots
            .iter()
            .map(|(symbol, snapshot)| (symbol.clone(), snapshot.price_limit)),
    );

    match SHARE.stock_snapshots.write() {
        Ok(mut cache) => {
            let mut changed_event_count = 0usize;
            // 先刪除這個類股上一輪有、這一輪沒有的股票，
            // 避免共享快取殘留已不在該類股結果中的舊資料。
            for symbol in previous_symbols {
                if !category_snapshots.contains_key(&symbol) {
                    cache.remove(&symbol);
                }
            }

            // 再把本輪抓到的快照逐筆寫回共享快取。
            // 若 symbol 已存在，就用最新 snapshot 覆蓋。
            for (symbol, snapshot) in category_snapshots {
                let price = snapshot.price;
                // Yahoo 興櫃類股的股票不一定已在主檔，改用類股本身的市場別決定閾值。
                let is_valid = if category.exchange == YahooClassExchange::Emerging {
                    SHARE.is_valid_price_for_market(&symbol, price, snapshot.last_close, true)
                } else {
                    SHARE.is_valid_price(&symbol, price, snapshot.last_close)
                };
                if !is_valid {
                    // 價格 0 是尚未成交（冷門股、特別股開盤後常見），不是異常，只略過不記錄
                    if price > Decimal::ZERO {
                        report_abnormal_price(
                            &symbol,
                            price,
                            snapshot.last_close,
                            &snapshot.source_site,
                        );
                    }
                    continue;
                }
                let has_changed = snapshot.price != Decimal::ZERO
                    && cache
                        .get(&symbol)
                        .is_none_or(|old_snapshot| old_snapshot.price != snapshot.price);
                let publish_symbol = has_changed.then(|| symbol.clone());
                cache.insert(symbol, snapshot);

                if let Some(symbol) = publish_symbol {
                    changed_event_count += 1;
                    trace_price_tasks::publish_price_update(symbol, price);
                }
            }

            // 回傳整體共享快取筆數，讓呼叫端能寫 log 觀察目前快取規模。
            ApplyCategoryResult {
                total_snapshot_count: cache.len(),
                changed_event_count,
            }
        }
        Err(why) => {
            // 寫鎖失敗時記錄錯誤，並回傳 0 讓 log 明顯顯示這輪更新沒有成功落地。
            tracing::error!("Failed to update Yahoo 類股快取 because {:?}", why);
            ApplyCategoryResult::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::super::TEST_STATE_LOCK;
    use super::*;

    /// 驗證快取全量更新前，只會針對價格實際異動的股票產生事件。
    #[test]
    fn test_apply_category_snapshots_counts_changed_prices() {
        let _lock = TEST_STATE_LOCK.blocking_lock();
        SHARE.clear_stock_snapshots();
        // 清空昨日收盤快取，防止並發測試（如 SHARE.load()）將真實 DB 價格寫入，
        // 導致 is_valid_price 以真實昨收價驗證測試用的假價格而誤判為異常。
        SHARE.clear_last_trading_day_quotes();

        let category = YahooClassCategory::enabled(
            crate::infra::crawler::yahoo::YahooClassExchange::Listed,
            40,
            "半導體",
        );
        let mut category_symbols = HashMap::new();

        let mut existing = HashMap::new();
        existing.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(998)),
        );
        SHARE.set_stock_snapshots(existing);

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

        let result = apply_category_snapshots(&category, new_data, &mut category_symbols);

        assert_eq!(result.changed_event_count, 2);

        SHARE.clear_stock_snapshots();
    }

    /// 驗證同一個類股重新更新時，已不存在的股票會從共用快取中移除。
    #[test]
    fn test_apply_category_snapshots_replaces_removed_symbols_in_same_category() {
        let _lock = TEST_STATE_LOCK.blocking_lock();
        SHARE.clear_stock_snapshots();
        SHARE.clear_last_trading_day_quotes();

        let category = YahooClassCategory::enabled(
            crate::infra::crawler::yahoo::YahooClassExchange::Listed,
            40,
            "半導體",
        );
        let mut category_symbols = HashMap::new();

        let mut first = HashMap::new();
        first.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(998)),
        );
        first.insert(
            "2303".to_string(),
            RealtimeSnapshot::new("2303".to_string(), dec!(45)),
        );
        apply_category_snapshots(&category, first, &mut category_symbols);

        let mut second = HashMap::new();
        second.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(999)),
        );
        apply_category_snapshots(&category, second, &mut category_symbols);

        let cache = SHARE.stock_snapshots.read().unwrap();
        assert!(cache.contains_key("2330"));
        assert!(!cache.contains_key("2303"));
    }
}
