//! HiStock 即時報價的測試。
//!
//! 子模組：`parsing`（解析）、`screening`（一致性篩選）、`task_cycle`（背景任務與快取）、
//! `live`（連線正式站，`#[ignore]`）；這裡放共用的 import 與假資料建構函式。

mod live;
mod parsing;
mod screening;
mod task_cycle;

use super::{parse::*, screen::*, task::*, *};
use crate::core::util::diagnostics::ProcessMemoryStats;
/// 與 Yahoo 類股快取、trace 價格任務共用：都會改動全域即時報價快取。
use crate::infra::cache::REALTIME_TEST_LOCK as TEST_STATE_LOCK;
use rust_decimal_macros::dec;
use scraper::{Html, Selector};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// 建立一列成交 78.5、漲 3.1、昨收 75.4、高 79、低 76 的正常快照。
pub(super) fn consistent_snapshot(symbol: &str) -> RealtimeSnapshot {
    let mut snapshot = RealtimeSnapshot::new(symbol.to_string(), dec!(78.5));
    snapshot.change = dec!(3.1);
    snapshot.last_close = dec!(75.4);
    snapshot.high = dec!(79);
    snapshot.low = dec!(76);
    snapshot
}

pub(super) fn fetch_result_with(snapshots: Vec<RealtimeSnapshot>) -> HiStockFetchResult {
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
