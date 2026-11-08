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

type DetailKey = (chrono::NaiveDate, u64, String, i64);
type HoldingKey = (chrono::NaiveDate, u64, Vec<String>);

/// 有容量與五分鐘 TTL 的共用 ETF 資料快取；同鍵未命中只執行一次查詢。
pub struct EtfAnalysisCache {
    generation: AtomicU64,
    details: Cache<DetailKey, EtfDetail>,
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

    /// ETF 明細；快取鍵含 `nav_days`，不同淨值筆數不會互相取代。錯誤與「查無這檔」都不快取，
    /// 剛上市的 ETF 在資料補齊後可立即查到。
    pub async fn detail(
        &self,
        symbol: &str,
        nav_days: i64,
        load: impl Future<Output = Result<Option<EtfDetail>>>,
    ) -> Result<Option<EtfDetail>> {
        let key = (
            Local::now().date_naive(),
            self.generation.load(Ordering::SeqCst),
            symbol.to_owned(),
            nav_days,
        );
        let loaded = self
            .details
            .try_get_with(key, async {
                load.await?.ok_or_else(|| anyhow::Error::new(NotFound))
            })
            .await;
        match loaded {
            Ok(detail) => Ok(Some(detail)),
            Err(error) if error.is::<NotFound>() => Ok(None),
            Err(error) => Err(shared_error(error)),
        }
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
            .map_err(shared_error)
    }
}

/// 載入器回傳「查無資料」的標記；不寫入快取，也不當成錯誤。
#[derive(Debug)]
struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not found")
    }
}

impl std::error::Error for NotFound {}

/// 多個等待者共用同一個載入錯誤；顯示與 `source()` 都沿用原始錯誤，保留完整的原因鏈。
#[derive(Debug)]
struct SharedError(Arc<anyhow::Error>);

impl std::fmt::Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.0, f)
    }
}

impl std::error::Error for SharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        (**self.0).source()
    }
}

pub(super) fn shared_error(error: Arc<anyhow::Error>) -> anyhow::Error {
    anyhow::Error::new(SharedError(error))
}

#[cfg(test)]
mod tests;
