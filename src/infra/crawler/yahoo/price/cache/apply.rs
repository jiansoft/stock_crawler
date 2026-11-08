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
        .insert(category_key, new_symbols.clone())
        .unwrap_or_default();

    // 驗證前先記下本輪的漲跌停價：Yahoo 類股報價自帶當日漲跌幅限制，比固定的 10.5% 門檻準確，
    // 也讓其他備援站點的報價能用同一份限制驗證。
    SHARE.set_price_limits(
        chrono::Local::now().date_naive(),
        category_snapshots
            .iter()
            .map(|(symbol, snapshot)| (symbol.clone(), snapshot.price_limit)),
    );

    // 驗價會讀其他快取並可能寫日誌，在取得共享快照寫鎖前完成。
    let valid_snapshots: Vec<_> = category_snapshots
        .into_iter()
        .filter(|(symbol, snapshot)| {
            let valid = if category.exchange == YahooClassExchange::Emerging {
                SHARE.is_valid_price_for_market(symbol, snapshot.price, snapshot.last_close, true)
            } else {
                SHARE.is_valid_price(symbol, snapshot.price, snapshot.last_close)
            };
            if !valid && snapshot.price > Decimal::ZERO {
                report_abnormal_price(
                    symbol,
                    snapshot.price,
                    snapshot.last_close,
                    &snapshot.source_site,
                );
            }
            valid
        })
        .collect();

    match SHARE.stock_snapshots.write() {
        Ok(mut cache) => {
            let mut changed_event_count = 0usize;
            // 先刪除這個類股上一輪有、這一輪沒有的股票，
            // 避免共享快取殘留已不在該類股結果中的舊資料。
            for symbol in previous_symbols {
                if !new_symbols.contains(&symbol) {
                    cache.remove(&symbol);
                }
            }

            // 再把本輪抓到的快照逐筆寫回共享快取。
            // 若 symbol 已存在，就用最新 snapshot 覆蓋。
            for (symbol, snapshot) in valid_snapshots {
                let price = snapshot.price;
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
mod tests;
