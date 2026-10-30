//! # 站點健康度 (Health)
//!
//! 追蹤各報價站點近期的成敗與耗時，決定每次輪詢的嘗試順序：
//!
//! - **熔斷**：連續失敗達 [`FAILURE_THRESHOLD`] 次就暫停該站，冷卻時間從
//!   [`BASE_COOLDOWN`] 起每次再熔斷加倍，最多 [`MAX_COOLDOWN`]。冷卻結束後的第一個結果
//!   決定恢復或立刻再熔斷（half-open）。
//! - **降級**：平均耗時（EWMA）明顯高於其他站的站點排到最後，只在前面都失敗時才用；
//!   每 [`PROBE_EVERY`] 次輪詢不降級一次，讓慢站有機會更新耗時而恢復。
//!
//! 2026-10-07 正式機：PcHome avg 2,031ms、p99 9,597ms，其餘站 avg 約 0.9～1.2 秒；
//! 當天 229 筆 `http.slow` 幾乎全來自 PcHome。
//!
//! 多檔股票並發抓價，站點熔斷的當下往往還有請求在路上；這些「熔斷前就送出」的請求
//! 結果只更新耗時，不影響熔斷狀態。否則一筆晚回來的成功就會立刻解除熔斷——
//! 2026-10-08 正式機的熔斷都在 3～30 毫秒內「恢復」，等於沒有作用。
//!
//! 冷門股開盤後尚未成交時各站都回「無價格」，這類失敗會平均落在每個站；
//! 只要其他股票有成功就會重設連續失敗數，不致讓正常站點熔斷。即使全部站點都熔斷，
//! [`plan_attempt_order`] 仍會回傳完整輪詢順序，不會整個斷線。

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Local;
use once_cell::sync::Lazy;

use crate::core::util::daily_seen::ConcurrentDailySeen;

/// 連續失敗幾次就熔斷。
const FAILURE_THRESHOLD: u32 = 5;
/// 第一次熔斷的冷卻時間。
const BASE_COOLDOWN: Duration = Duration::from_secs(30);
/// 冷卻時間上限。
const MAX_COOLDOWN: Duration = Duration::from_secs(300);
/// EWMA 平滑係數（新樣本權重）。
const EWMA_ALPHA: f64 = 0.2;
/// EWMA 至少高於其餘站點中位數幾倍才算慢站。
const SLOW_RATIO: f64 = 1.8;
/// EWMA 低於此值（毫秒）一律不算慢站，避免整體都很快時為了幾十毫秒的差距降級。
const SLOW_FLOOR_MS: f64 = 1_000.0;
/// 每幾次輪詢略過一次降級，讓慢站累積新樣本。
const PROBE_EVERY: usize = 20;

/// 全部站點的健康狀態。
static HEALTH: Lazy<Mutex<HealthBook>> = Lazy::new(|| Mutex::new(HealthBook::default()));
/// 輪詢序號，用來決定哪一次要略過降級（探測）。
static PLAN_SEQ: AtomicUsize = AtomicUsize::new(0);
/// 今天已熔斷過的站點：每站每天第一次熔斷記 WARN，之後記 INFO（一天可能熔斷上百次）。
static TRIPPED_TODAY: Lazy<ConcurrentDailySeen> = Lazy::new(ConcurrentDailySeen::default);

/// 單一站點的健康狀態。
#[derive(Debug, Default)]
struct SiteHealth {
    /// 目前連續失敗次數，任何一次成功就歸零。
    consecutive_failures: u32,
    /// 連續熔斷次數，決定下一次冷卻時間；恢復後歸零。
    trips: u32,
    /// 熔斷到這個時間點為止；`None` 表示未熔斷。
    open_until: Option<Instant>,
    /// 最近一次熔斷的時間點；在此之前送出的請求結果不影響熔斷狀態。
    tripped_at: Option<Instant>,
    /// 平均耗時（毫秒，EWMA）；尚無樣本時為 `None`。
    ewma_ms: Option<f64>,
}

/// 單次結果造成的狀態轉換，交由呼叫端記錄日誌。
#[derive(Debug, PartialEq, Eq)]
enum Transition {
    /// 熔斷，附冷卻時間。
    Tripped(Duration),
    /// 熔斷後第一次成功，恢復正常。
    Recovered,
}

impl SiteHealth {
    /// 目前是否處於熔斷冷卻中。
    fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|until| now < until)
    }

    /// 記錄一次結果並更新耗時。
    ///
    /// `started_at` 是請求送出的時間：早於最近一次熔斷的結果只更新耗時。
    fn record(&mut self, ok: bool, started_at: Instant, now: Instant) -> Option<Transition> {
        let sample = now.saturating_duration_since(started_at).as_millis() as f64;
        self.ewma_ms = Some(match self.ewma_ms {
            Some(prev) => prev + EWMA_ALPHA * (sample - prev),
            None => sample,
        });

        if self
            .tripped_at
            .is_some_and(|tripped_at| started_at < tripped_at)
        {
            return None;
        }

        if ok {
            self.consecutive_failures = 0;
            let recovered = self.trips > 0;
            self.trips = 0;
            self.open_until = None;
            return recovered.then_some(Transition::Recovered);
        }

        self.consecutive_failures += 1;
        // 冷卻剛結束（half-open）時第一次失敗就直接再熔斷，不必重新累積。
        let half_open = self.trips > 0 && !self.is_open(now);
        if self.consecutive_failures < FAILURE_THRESHOLD && !half_open {
            return None;
        }

        self.trips += 1;
        self.consecutive_failures = 0;
        let cooldown = cooldown_for(self.trips);
        self.open_until = Some(now + cooldown);
        self.tripped_at = Some(now);
        Some(Transition::Tripped(cooldown))
    }
}

/// 第 `trips` 次連續熔斷的冷卻時間：30s、60s、120s…，最多 5 分鐘。
fn cooldown_for(trips: u32) -> Duration {
    let factor = 2u32.saturating_pow(trips.saturating_sub(1));
    BASE_COOLDOWN.saturating_mul(factor).min(MAX_COOLDOWN)
}

/// 全部站點的健康狀態表。
#[derive(Debug, Default)]
struct HealthBook {
    sites: HashMap<&'static str, SiteHealth>,
}

impl HealthBook {
    fn record(
        &mut self,
        name: &'static str,
        ok: bool,
        started_at: Instant,
        now: Instant,
    ) -> Option<Transition> {
        self.sites
            .entry(name)
            .or_default()
            .record(ok, started_at, now)
    }

    /// 排出本次的嘗試順序（回傳 `names` 的索引）。
    ///
    /// 從 `start` 開始輪詢，略過熔斷中的站點；`demote_slow` 為真時把慢站移到最後。
    /// 全部站點都熔斷時回傳完整輪詢順序。
    fn plan(
        &self,
        names: &[&'static str],
        start: usize,
        now: Instant,
        demote_slow: bool,
    ) -> Vec<usize> {
        let len = names.len();
        let rotation = (0..len).map(|offset| (start + offset) % len);

        let available = rotation
            .clone()
            .filter(|&idx| !self.sites.get(names[idx]).is_some_and(|h| h.is_open(now)))
            .collect::<Vec<_>>();
        if available.is_empty() {
            return rotation.collect();
        }
        if !demote_slow {
            return available;
        }

        let (slow, fast): (Vec<usize>, Vec<usize>) = available
            .into_iter()
            .partition(|&idx| self.is_slow(names, idx));
        fast.into_iter().chain(slow).collect()
    }

    /// 站點的平均耗時是否明顯高於同池其他站點的中位數。
    fn is_slow(&self, names: &[&'static str], idx: usize) -> bool {
        let Some(own) = self.sites.get(names[idx]).and_then(|h| h.ewma_ms) else {
            return false;
        };
        if own < SLOW_FLOOR_MS {
            return false;
        }

        let mut others = names
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != idx)
            .filter_map(|(_, name)| self.sites.get(name).and_then(|h| h.ewma_ms))
            .collect::<Vec<_>>();
        if others.is_empty() {
            return false;
        }
        others.sort_by(f64::total_cmp);
        let median = others[others.len() / 2];
        own > median * SLOW_RATIO
    }
}

/// 排出本次輪詢的嘗試順序（回傳 `names` 的索引）。
///
/// 每個索引只出現一次；`start` 由呼叫端的全域遊標決定，確保不同請求從不同站開始。
pub(super) fn plan_attempt_order(names: &[&'static str], start: usize) -> Vec<usize> {
    let demote_slow = !PLAN_SEQ
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(PROBE_EVERY);
    match HEALTH.lock() {
        Ok(book) => book.plan(names, start, Instant::now(), demote_slow),
        // 鎖中毒時退回單純輪詢，不影響抓價。
        Err(_) => (0..names.len())
            .map(|offset| (start + offset) % names.len())
            .collect(),
    }
}

/// 記錄站點本次請求的成敗與耗時，回傳這次是否讓站點熔斷。
///
/// 熔斷時寫日誌：每站每天第一次記 WARN，之後記 INFO（熔斷次數另計入收盤的站點統計）；恢復記 INFO。
pub(super) fn record_site_outcome(name: &'static str, ok: bool, started_at: Instant) -> bool {
    let transition = match HEALTH.lock() {
        Ok(mut book) => book.record(name, ok, started_at, Instant::now()),
        Err(_) => return false,
    };

    match transition {
        Some(Transition::Tripped(cooldown)) => {
            let secs = cooldown.as_secs();
            if TRIPPED_TODAY.first_today(name, Local::now().date_naive()) {
                tracing::warn!(
                    site = name,
                    cooldown_secs = secs,
                    "報價站點今天第一次連續失敗熔斷，暫停 {secs} 秒：{name}（之後的熔斷記 INFO，次數見收盤站點統計）"
                );
            } else {
                tracing::info!(
                    site = name,
                    cooldown_secs = secs,
                    "報價站點連續失敗，暫停 {secs} 秒：{name}"
                );
            }
            true
        }
        Some(Transition::Recovered) => {
            tracing::info!(site = name, "報價站點恢復：{}", name);
            false
        }
        None => false,
    }
}

#[cfg(test)]
mod tests;
