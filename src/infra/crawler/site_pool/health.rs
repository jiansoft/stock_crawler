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

use once_cell::sync::Lazy;

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

/// 單一站點的健康狀態。
#[derive(Debug, Default)]
struct SiteHealth {
    /// 目前連續失敗次數，任何一次成功就歸零。
    consecutive_failures: u32,
    /// 連續熔斷次數，決定下一次冷卻時間；恢復後歸零。
    trips: u32,
    /// 熔斷到這個時間點為止；`None` 表示未熔斷。
    open_until: Option<Instant>,
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
    fn record(&mut self, ok: bool, elapsed_ms: u64, now: Instant) -> Option<Transition> {
        let sample = elapsed_ms as f64;
        self.ewma_ms = Some(match self.ewma_ms {
            Some(prev) => prev + EWMA_ALPHA * (sample - prev),
            None => sample,
        });

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
        elapsed_ms: u64,
        now: Instant,
    ) -> Option<Transition> {
        self.sites
            .entry(name)
            .or_default()
            .record(ok, elapsed_ms, now)
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

/// 記錄站點本次請求的成敗與耗時，熔斷或恢復時寫 info 日誌。
pub(super) fn record_site_outcome(name: &'static str, ok: bool, started_at: Instant) {
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    let transition = match HEALTH.lock() {
        Ok(mut book) => book.record(name, ok, elapsed_ms, Instant::now()),
        Err(_) => return,
    };

    match transition {
        Some(Transition::Tripped(cooldown)) => tracing::info!(
            site = name,
            cooldown_secs = cooldown.as_secs(),
            "報價站點連續失敗，暫停 {} 秒：{}",
            cooldown.as_secs(),
            name
        ),
        Some(Transition::Recovered) => {
            tracing::info!(site = name, "報價站點恢復：{}", name)
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 4] = ["A", "B", "C", "D"];

    fn fail_times(book: &mut HealthBook, name: &'static str, times: u32, now: Instant) {
        for _ in 0..times {
            book.record(name, false, 100, now);
        }
    }

    /// 沒有任何紀錄時就是單純從 start 開始的輪詢，每站各一次。
    #[test]
    fn plan_without_history_is_plain_rotation() {
        let book = HealthBook::default();
        assert_eq!(book.plan(&NAMES, 2, Instant::now(), true), vec![2, 3, 0, 1]);
    }

    /// 連續失敗達門檻才熔斷；中間有一次成功就重新計算。
    #[test]
    fn site_trips_after_consecutive_failures() {
        let mut book = HealthBook::default();
        let now = Instant::now();

        fail_times(&mut book, "B", FAILURE_THRESHOLD - 1, now);
        book.record("B", true, 100, now);
        fail_times(&mut book, "B", FAILURE_THRESHOLD - 1, now);
        assert_eq!(book.plan(&NAMES, 0, now, true), vec![0, 1, 2, 3]);

        assert_eq!(
            book.record("B", false, 100, now),
            Some(Transition::Tripped(BASE_COOLDOWN))
        );
        assert_eq!(book.plan(&NAMES, 0, now, true), vec![0, 2, 3]);
    }

    /// 冷卻結束後第一次就失敗會立刻再熔斷且冷卻加倍；成功則恢復。
    #[test]
    fn half_open_failure_doubles_cooldown_and_success_recovers() {
        let mut book = HealthBook::default();
        let now = Instant::now();
        fail_times(&mut book, "C", FAILURE_THRESHOLD, now);

        let after_first = now + BASE_COOLDOWN + Duration::from_secs(1);
        assert_eq!(book.plan(&NAMES, 0, after_first, true), vec![0, 1, 2, 3]);
        assert_eq!(
            book.record("C", false, 100, after_first),
            Some(Transition::Tripped(BASE_COOLDOWN * 2))
        );
        assert_eq!(book.plan(&NAMES, 0, after_first, true), vec![0, 1, 3]);

        let after_second = after_first + BASE_COOLDOWN * 2 + Duration::from_secs(1);
        assert_eq!(
            book.record("C", true, 100, after_second),
            Some(Transition::Recovered)
        );
        assert_eq!(book.plan(&NAMES, 0, after_second, true), vec![0, 1, 2, 3]);
    }

    /// 冷卻時間逐次加倍並封頂。
    #[test]
    fn cooldown_is_capped() {
        assert_eq!(cooldown_for(1), Duration::from_secs(30));
        assert_eq!(cooldown_for(2), Duration::from_secs(60));
        assert_eq!(cooldown_for(4), Duration::from_secs(240));
        assert_eq!(cooldown_for(5), MAX_COOLDOWN);
        assert_eq!(cooldown_for(40), MAX_COOLDOWN);
    }

    /// 全部熔斷時仍回傳完整輪詢，不讓報價整個中斷。
    #[test]
    fn all_open_falls_back_to_full_rotation() {
        let mut book = HealthBook::default();
        let now = Instant::now();
        for name in NAMES {
            fail_times(&mut book, name, FAILURE_THRESHOLD, now);
        }
        assert_eq!(book.plan(&NAMES, 1, now, true), vec![1, 2, 3, 0]);
    }

    /// 明顯較慢的站點排到最後；探測輪次不降級；整體都快時不降級。
    #[test]
    fn slow_site_is_demoted_except_on_probe() {
        let mut book = HealthBook::default();
        let now = Instant::now();
        for name in ["A", "B", "C"] {
            book.record(name, true, 900, now);
        }
        book.record("D", true, 2_000, now);
        book.record("A", true, 950, now);

        assert_eq!(book.plan(&NAMES, 3, now, true), vec![0, 1, 2, 3]);
        assert_eq!(book.plan(&NAMES, 3, now, false), vec![3, 0, 1, 2]);

        let mut quick = HealthBook::default();
        for name in ["A", "B", "C"] {
            quick.record(name, true, 100, now);
        }
        quick.record("D", true, 500, now);
        assert_eq!(quick.plan(&NAMES, 3, now, true), vec![3, 0, 1, 2]);
    }

    /// EWMA 會往新樣本靠近，慢站變快後不再降級。
    #[test]
    fn slow_site_recovers_when_latency_drops() {
        let mut book = HealthBook::default();
        let now = Instant::now();
        for name in ["A", "B", "C"] {
            book.record(name, true, 900, now);
        }
        book.record("D", true, 3_000, now);
        assert!(book.is_slow(&NAMES, 3));

        for _ in 0..20 {
            book.record("D", true, 900, now);
        }
        assert!(!book.is_slow(&NAMES, 3));
    }
}
