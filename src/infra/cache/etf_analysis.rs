//! ETF 比較與持股穿透共用的短期查詢快取；個人持股與市值每次重新讀取。

use crate::domain::{etf::query::EtfDetail, portfolio::exposure::EtfConstituent};
use anyhow::Result;
use chrono::Local;
use moka::future::Cache;
use once_cell::sync::Lazy;
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

type DetailKey = (chrono::NaiveDate, u64, String);
type HoldingKey = (chrono::NaiveDate, u64, Vec<String>);

/// 有容量與五分鐘 TTL 的共用 ETF 資料快取；同鍵未命中只執行一次查詢。
pub struct EtfAnalysisCache {
    generation: AtomicU64,
    details: Cache<DetailKey, Option<EtfDetail>>,
    holdings: Cache<HoldingKey, Vec<EtfConstituent>>,
}

impl Default for EtfAnalysisCache {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            details: Cache::builder()
                .max_capacity(512)
                .time_to_live(Duration::from_secs(300))
                .build(),
            holdings: Cache::builder()
                .max_capacity(512)
                .time_to_live(Duration::from_secs(300))
                .build(),
        }
    }
}

/// 全域二級查詢快取。
pub static ETF_ANALYSIS: Lazy<EtfAnalysisCache> = Lazy::new(EtfAnalysisCache::default);

impl EtfAnalysisCache {
    /// 成功寫入 ETF 資料後推進世代；進行中的舊查詢無法污染新世代。
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.details.invalidate_all();
        self.holdings.invalidate_all();
    }

    /// 比較用明細（固定一筆淨值）；錯誤不快取。
    pub async fn detail(
        &self,
        symbol: &str,
        load: impl Future<Output = Result<Option<EtfDetail>>>,
    ) -> Result<Option<EtfDetail>> {
        let key = (
            Local::now().date_naive(),
            self.generation.load(Ordering::SeqCst),
            symbol.to_owned(),
        );
        self.details
            .try_get_with(key, load)
            .await
            .map_err(|error: Arc<anyhow::Error>| anyhow::anyhow!("{error:#}"))
    }

    /// 成分股快取依正規化代號組合分隔，不保存會員資料。
    pub async fn constituents(
        &self,
        symbols: &[String],
        load: impl Future<Output = Result<Vec<EtfConstituent>>>,
    ) -> Result<Vec<EtfConstituent>> {
        let mut symbols = symbols.to_vec();
        symbols.sort();
        symbols.dedup();
        let key = (
            Local::now().date_naive(),
            self.generation.load(Ordering::SeqCst),
            symbols,
        );
        self.holdings
            .try_get_with(key, load)
            .await
            .map_err(|error: Arc<anyhow::Error>| anyhow::anyhow!("{error:#}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn coalesces_misses_and_invalidates_without_caching_errors() {
        use std::sync::atomic::AtomicUsize;
        let cache = EtfAnalysisCache::default();
        let calls = AtomicUsize::new(0);
        let load = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok(None)
        };
        let (a, b) = tokio::join!(cache.detail("0050", load()), cache.detail("0050", load()));
        assert!(a.unwrap().is_none() && b.unwrap().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.invalidate();
        assert!(
            cache
                .detail("0050", async { Err(anyhow::anyhow!("失敗")) })
                .await
                .is_err()
        );
        cache.detail("0050", load()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
