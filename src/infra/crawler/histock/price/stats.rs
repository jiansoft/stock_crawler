//! HiStock 每日背景採集統計；逐列異常留待收盤彙總，避免單筆警告洗版。

use chrono::{Local, NaiveDate, NaiveTime};
use once_cell::sync::Lazy;
use std::{collections::BTreeSet, sync::Mutex};

use super::screen::Inconsistency;

/// 每天最多保存的耗時樣本；五秒一輪即使全天執行仍不超過此值。
const MAX_SAMPLES: usize = 20_000;
/// 排行頁整輪耗時達十秒算慢回應。
const SLOW_MS: u64 = 10_000;
/// 當天的累積資料，跨日自動更換，收盤輸出後清空樣本。
static DAILY: Lazy<Mutex<DailyStats>> = Lazy::new(|| Mutex::new(DailyStats::default()));

/// 一天的輪數、成功率、延遲分布與矛盾列統計。
#[derive(Default)]
struct DailyStats {
    date: Option<NaiveDate>,
    rounds: u64,
    succeeded: u64,
    slow: u64,
    elapsed: Vec<u64>,
    inconsistent: usize,
    /// 矛盾列次依原因：成交價不在高低間、漲跌對不上昨收、疑似別檔。
    out_of_range: usize,
    change_mismatch: usize,
    misaligned: usize,
    symbols: BTreeSet<String>,
    /// 整批捨棄的輪數與第一輪、最後一輪的時間（開盤那段 HiStock 整頁不可用多久）。
    dirty_batches: u64,
    dirty_window: Option<(NaiveTime, NaiveTime)>,
}

impl DailyStats {
    /// 日期不同時換成新一天的資料，避免前一交易日混入。
    fn on_day(&mut self, today: NaiveDate) {
        if self.date != Some(today) {
            *self = Self {
                date: Some(today),
                ..Self::default()
            };
        }
    }

    /// 記錄一輪（包含失敗）；樣本有上限，計數仍涵蓋全部輪數。
    fn cycle(&mut self, succeeded: bool, elapsed_ms: u64) {
        self.rounds += 1;
        self.succeeded += u64::from(succeeded);
        self.slow += u64::from(elapsed_ms >= SLOW_MS);
        if self.elapsed.len() < MAX_SAMPLES {
            self.elapsed.push(elapsed_ms);
        }
    }

    /// 記一列矛盾列。
    fn inconsistent_row(&mut self, symbol: &str, reason: Inconsistency) {
        self.inconsistent += 1;
        match reason {
            Inconsistency::OutOfRange => self.out_of_range += 1,
            Inconsistency::ChangeMismatch => self.change_mismatch += 1,
            Inconsistency::Misaligned => self.misaligned += 1,
        }
        self.symbols.insert(symbol.to_owned());
    }

    /// 記一輪整批捨棄的時間。
    fn dirty_batch(&mut self, at: NaiveTime) {
        self.dirty_batches += 1;
        self.dirty_window = Some(match self.dirty_window {
            Some((first, last)) => (first.min(at), last.max(at)),
            None => (at, at),
        });
    }

    /// 產生收盤統計文字；百分位使用排序後的 nearest-rank。
    fn line(&self) -> String {
        let mut samples = self.elapsed.clone();
        samples.sort_unstable();
        let index = (samples.len() * 99).div_ceil(100).saturating_sub(1);
        let p99 = samples.get(index).copied().unwrap_or(0);
        let success = if self.rounds == 0 {
            0.0
        } else {
            self.succeeded as f64 / self.rounds as f64 * 100.0
        };
        let symbols: Vec<_> = self.symbols.iter().take(5).map(String::as_str).collect();
        let window = self
            .dirty_window
            .map(|(first, last)| {
                format!(
                    "({}～{})",
                    first.format("%H:%M:%S"),
                    last.format("%H:%M:%S")
                )
            })
            .unwrap_or_default();
        format!(
            "HiStock 收盤統計：rounds={}, succeeded={}, success={success:.1}%, slow={}, p99={p99}ms, inconsistent={}(高低 {}、漲跌 {}、別檔 {}), dirty_batches={}{window}, symbols={}",
            self.rounds,
            self.succeeded,
            self.slow,
            self.inconsistent,
            self.out_of_range,
            self.change_mismatch,
            self.misaligned,
            self.dirty_batches,
            symbols.join(",")
        )
    }
}

/// 背景迴圈完成一次請求後記錄，耗時包含正常重試。
pub(super) fn record_cycle(succeeded: bool, elapsed_ms: u64) {
    if let Ok(mut stats) = DAILY.lock() {
        stats.on_day(Local::now().date_naive());
        stats.cycle(succeeded, elapsed_ms);
    }
}

/// 篩選器記錄當輪所有矛盾列，包括整批捨棄；計數按列次累計，代號去重。
pub(super) fn record_inconsistent<'a>(rows: impl Iterator<Item = (&'a str, Inconsistency)>) {
    if let Ok(mut stats) = DAILY.lock() {
        stats.on_day(Local::now().date_naive());
        for (symbol, reason) in rows {
            stats.inconsistent_row(symbol, reason);
        }
    }
}

/// 背景迴圈遇到整批捨棄時記錄當下時間。
pub(super) fn record_dirty_batch() {
    if let Ok(mut stats) = DAILY.lock() {
        let now = Local::now();
        stats.on_day(now.date_naive());
        stats.dirty_batch(now.time());
    }
}

/// 收盤停止背景迴圈後輸出一次；重複停止不重複輸出同一批統計。
pub(super) fn summarize() {
    if let Ok(mut stats) = DAILY.lock() {
        if stats.rounds > 0 {
            tracing::info!("{}", stats.line());
        }
        *stats = DailyStats::default();
    }
}

#[cfg(test)]
mod tests;
