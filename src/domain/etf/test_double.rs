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
    EtfHoldingPeriods, EtfNavHistoryPoint, EtfNavRecord, EtfPerformance, EtfPortfolio,
    EtfProfileRecord, EtfRepository, EtfWeeklyFlow, PremiumPoint, UnitsChangePoint,
};
use crate::domain::quote::technical::PriceBar;

/// 回傳固定的上市 ETF 清單，記錄歷史淨值、基本資料與持股的寫入，以及快照重建次數。
#[derive(Default)]
pub(crate) struct RecordingEtfRepository {
    /// `listed_etfs` 回傳的代號（名稱一律空字串）。
    pub(crate) listed: Vec<String>,
    /// 歷次 `insert_nav_history` 的代號與筆數。
    pub(crate) inserted: Mutex<Vec<(String, usize)>>,
    /// 歷次 `save_profiles` 寫入的代號（每次一批）。
    pub(crate) profile_batches: Mutex<Vec<Vec<String>>>,
    /// 歷次 `replace_portfolio` 的代號。
    pub(crate) portfolios: Mutex<Vec<String>>,
    /// `refresh_snapshot` 的呼叫次數。
    pub(crate) snapshots: AtomicUsize,
    /// `weekly_flows` 回傳的資料。
    pub(crate) weekly: Vec<EtfWeeklyFlow>,
    /// `holding_periods` 回傳的資料（依代號過濾）。
    pub(crate) periods: Vec<EtfHoldingPeriods>,
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

    /// 歷次 `save_profiles` 寫入的代號（每次一批）。
    pub(crate) fn profile_batches(&self) -> Vec<Vec<String>> {
        self.profile_batches.lock().expect("鎖未中毒").clone()
    }

    /// 歷次 `replace_portfolio` 的代號。
    pub(crate) fn portfolios(&self) -> Vec<String> {
        self.portfolios.lock().expect("鎖未中毒").clone()
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

    async fn save_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64> {
        self.profile_batches.lock().expect("鎖未中毒").push(
            profiles
                .iter()
                .map(|profile| profile.stock_symbol.clone())
                .collect(),
        );
        Ok(profiles.len() as u64)
    }

    async fn insert_missing_profiles(&self, _profiles: &[EtfProfileRecord]) -> Result<u64> {
        unimplemented!()
    }

    async fn replace_portfolio(&self, portfolio: &EtfPortfolio) -> Result<u64> {
        self.portfolios
            .lock()
            .expect("鎖未中毒")
            .push(portfolio.stock_symbol.clone());
        Ok(portfolio.holdings.len() as u64)
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

    async fn holding_periods(
        &self,
        stock_symbols: &[String],
        _recorded_since: NaiveDate,
    ) -> Result<Vec<EtfHoldingPeriods>> {
        Ok(self
            .periods
            .iter()
            .filter(|p| stock_symbols.contains(&p.stock_symbol))
            .cloned()
            .collect())
    }

    async fn weekly_flows(&self, _days: i64) -> Result<Vec<EtfWeeklyFlow>> {
        Ok(self.weekly.clone())
    }

    async fn latest_units(
        &self,
        _stock_symbols: &[String],
    ) -> Result<HashMap<String, UnitsChangePoint>> {
        unimplemented!()
    }

    async fn recent_premiums(
        &self,
        _stock_symbols: &[String],
        _days: i64,
    ) -> Result<HashMap<String, Vec<PremiumPoint>>> {
        unimplemented!()
    }
}
