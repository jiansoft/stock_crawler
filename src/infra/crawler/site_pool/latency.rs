//! # 站點延遲統計 (Latency)
//!
//! 此子模組負責累積各報價站點的單次請求耗時，並在收盤後輸出
//! 取樣次數、平均耗時與 `p50`、`p70`、`p99` 百分位延遲的人類可讀摘要。

use std::{collections::HashMap, sync::Mutex, time::Instant};

use once_cell::sync::Lazy;

/// 各報價站點的延遲統計（供收盤後輸出人類可讀的追蹤資訊）。
static SITE_LATENCY_STATS: Lazy<Mutex<HashMap<&'static str, SiteLatencyStats>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 單一站點在目前統計期間內累積的延遲樣本。
#[derive(Default)]
struct SiteLatencyStats {
    /// 每次請求的耗時，單位為毫秒。
    durations_ms: Vec<u64>,
}

/// 單一站點延遲統計的彙總快照。
///
/// 此結構只在輸出 log 前短暫建立，用來承載排序後的摘要資訊。
struct SiteLatencySnapshot {
    /// 站點名稱。
    site_name: &'static str,
    /// 取樣次數。
    count: usize,
    /// 平均延遲，單位為毫秒。
    avg_ms: u64,
    /// 第 50 百分位延遲，單位為毫秒。
    p50_ms: u64,
    /// 第 70 百分位延遲，單位為毫秒。
    p70_ms: u64,
    /// 第 99 百分位延遲，單位為毫秒。
    p99_ms: u64,
}

impl SiteLatencyStats {
    /// 新增一筆站點延遲樣本。
    fn record(&mut self, elapsed_ms: u64) {
        self.durations_ms.push(elapsed_ms);
    }

    /// 取得目前累積的樣本數。
    fn sample_count(&self) -> usize {
        self.durations_ms.len()
    }

    /// 計算平均延遲，單位為毫秒。
    fn average_ms(&self) -> u64 {
        if self.durations_ms.is_empty() {
            return 0;
        }

        let sum: u128 = self.durations_ms.iter().map(|v| u128::from(*v)).sum();
        (sum / self.durations_ms.len() as u128) as u64
    }

    /// 計算指定百分位延遲，單位為毫秒。
    ///
    /// # 參數
    /// - `percentile`: 百分位數，範圍應介於 `1..=100`。
    ///
    /// 若樣本不足對應百分位所需數量，會回傳排序後最接近該百分位的樣本值。
    fn percentile_ms(&self, percentile: usize) -> u64 {
        if self.durations_ms.is_empty() {
            return 0;
        }

        let mut values = self.durations_ms.clone();
        values.sort_unstable();

        let len = values.len();
        let percentile = percentile.clamp(1, 100);
        let idx = (len * percentile).div_ceil(100).saturating_sub(1);
        values[idx]
    }

    /// 計算第 50 百分位延遲，單位為毫秒。
    fn p50_ms(&self) -> u64 {
        self.percentile_ms(50)
    }

    /// 計算第 70 百分位延遲，單位為毫秒。
    fn p70_ms(&self) -> u64 {
        self.percentile_ms(70)
    }

    /// 計算第 99 百分位延遲，單位為毫秒。
    ///
    /// 若樣本不足 100 筆，會回傳排序後靠近尾端的樣本值，
    /// 作為保守的高延遲觀察指標。
    fn p99_ms(&self) -> u64 {
        self.percentile_ms(99)
    }
}

/// 記錄單一站點本次請求耗時。
pub(super) fn record_site_latency(site_name: &'static str, started_at: Instant) {
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if let Ok(mut stats) = SITE_LATENCY_STATS.lock() {
        stats.entry(site_name).or_default().record(elapsed_ms);
    }
}

/// 輸出站點耗時統計並清空當前累積資料。
///
/// 供收盤事件呼叫，將當日 `fetch_stock_price_from_remote_site` 與
/// `fetch_stock_quotes_from_remote_site` 的站點耗時統一輸出。
/// 摘要欄位包含取樣次數、平均耗時，以及 `p50`、`p70`、`p99` 百分位延遲。
pub fn flush_site_latency_stats() {
    let mut stats = match SITE_LATENCY_STATS.lock() {
        Ok(guard) => guard,
        Err(_) => {
            tracing::error!("Failed to lock site latency stats for flush");
            return;
        }
    };

    if stats.is_empty() {
        tracing::info!("站點延遲統計: 無資料");
        return;
    }

    let mut entries = stats
        .iter()
        .map(|(site_name, site_stats)| SiteLatencySnapshot {
            site_name,
            count: site_stats.sample_count(),
            avg_ms: site_stats.average_ms(),
            p50_ms: site_stats.p50_ms(),
            p70_ms: site_stats.p70_ms(),
            p99_ms: site_stats.p99_ms(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.p99_ms
            .cmp(&right.p99_ms)
            .then(left.avg_ms.cmp(&right.avg_ms))
            .then(left.site_name.cmp(right.site_name))
    });

    for entry in entries {
        tracing::info!(
            "站點整體耗時統計 {}: count={}, avg={}ms, p50={}ms, p70={}ms, p99={}ms",
            entry.site_name,
            entry.count,
            entry.avg_ms,
            entry.p50_ms,
            entry.p70_ms,
            entry.p99_ms
        );
    }

    stats.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驗證站點延遲統計可以正確計算平均值與各主要百分位。
    #[test]
    fn test_site_latency_stats_average_and_percentiles() {
        let mut stats = SiteLatencyStats::default();

        for elapsed_ms in [10, 20, 30, 40, 50] {
            stats.record(elapsed_ms);
        }

        assert_eq!(stats.sample_count(), 5);
        assert_eq!(stats.average_ms(), 30);
        assert_eq!(stats.p50_ms(), 30);
        assert_eq!(stats.p70_ms(), 40);
        assert_eq!(stats.p99_ms(), 50);
    }

    /// 驗證站點延遲統計在單一樣本時仍能正確回傳平均值與各主要百分位。
    #[test]
    fn test_site_latency_stats_percentiles_with_single_sample() {
        let mut stats = SiteLatencyStats::default();
        stats.record(88);

        assert_eq!(stats.sample_count(), 1);
        assert_eq!(stats.average_ms(), 88);
        assert_eq!(stats.p50_ms(), 88);
        assert_eq!(stats.p70_ms(), 88);
        assert_eq!(stats.p99_ms(), 88);
    }

    /// 驗證輸出延遲統計後，累積中的站點資料會被清空。
    #[test]
    fn test_flush_site_latency_stats_clears_data() {
        {
            let mut all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
            all_stats.clear();
            all_stats.entry("Yahoo").or_default().record(12);
            all_stats.entry("Fugle").or_default().record(34);
        }

        flush_site_latency_stats();

        let all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
        assert!(all_stats.is_empty());
    }
}
