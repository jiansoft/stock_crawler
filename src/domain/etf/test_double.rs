//! ETF 領域的測試替身（test double）。
//!
//! 只在 `cfg(test)` 下編譯，供各層測試共用，不屬於產品程式碼。
//! [`EtfRepository`] 方法多，用不到的一律 `unimplemented!()`；獨立成檔讓覆蓋率設定（`codecov.yml`）整檔排除。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::{
    EtfNavHistoryPoint, EtfNavRecord, EtfPerformance, EtfPortfolio, EtfProfileRecord,
    EtfRepository, EtfWeeklyFlow, PremiumPoint,
};
use crate::domain::quote::technical::PriceBar;

/// 回傳固定的上市 ETF 清單，記錄歷史淨值的寫入與快照重建次數。
#[derive(Default)]
pub(crate) struct RecordingEtfRepository {
    /// `listed_etfs` 回傳的代號（名稱一律空字串）。
    pub(crate) listed: Vec<String>,
    /// 歷次 `insert_nav_history` 的代號與筆數。
    pub(crate) inserted: Mutex<Vec<(String, usize)>>,
    /// `refresh_snapshot` 的呼叫次數。
    pub(crate) snapshots: AtomicUsize,
    /// `weekly_flows` 回傳的資料。
    pub(crate) weekly: Vec<EtfWeeklyFlow>,
}

impl RecordingEtfRepository {
    /// 建立回傳指定上市 ETF 的替身。
    pub(crate) fn with_listed(symbols: &[&str]) -> Self {
        Self {
            listed: symbols.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// 歷次寫入的代號與筆數。
    pub(crate) fn inserted(&self) -> Vec<(String, usize)> {
        self.inserted.lock().expect("鎖未中毒").clone()
    }

    /// 快照重建次數。
    pub(crate) fn snapshots(&self) -> usize {
        self.snapshots.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EtfRepository for RecordingEtfRepository {
    async fn listed_etfs(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .listed
            .iter()
            .map(|s| (s.clone(), String::new()))
            .collect())
    }

    async fn save_navs(&self, _records: &[EtfNavRecord]) -> Result<u64> {
        unimplemented!()
    }

    async fn insert_nav_history(
        &self,
        stock_symbol: &str,
        points: &[EtfNavHistoryPoint],
    ) -> Result<u64> {
        self.inserted
            .lock()
            .expect("鎖未中毒")
            .push((stock_symbol.to_string(), points.len()));
        Ok(points.len() as u64)
    }

    async fn save_profiles(&self, _profiles: &[EtfProfileRecord]) -> Result<u64> {
        unimplemented!()
    }

    async fn insert_missing_profiles(&self, _profiles: &[EtfProfileRecord]) -> Result<u64> {
        unimplemented!()
    }

    async fn replace_portfolio(&self, _portfolio: &EtfPortfolio) -> Result<u64> {
        unimplemented!()
    }

    async fn nav_series(
        &self,
        _stock_symbols: &[String],
        _from: NaiveDate,
    ) -> Result<HashMap<String, Vec<PriceBar>>> {
        unimplemented!()
    }

    async fn save_performances(&self, _performances: &[EtfPerformance]) -> Result<u64> {
        unimplemented!()
    }

    async fn refresh_snapshot(&self) -> Result<u64> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        Ok(self.listed.len() as u64)
    }

    async fn weekly_flows(&self, _days: i64) -> Result<Vec<EtfWeeklyFlow>> {
        Ok(self.weekly.clone())
    }

    async fn recent_premiums(
        &self,
        _stock_symbols: &[String],
        _days: i64,
    ) -> Result<HashMap<String, Vec<PremiumPoint>>> {
        unimplemented!()
    }
}
