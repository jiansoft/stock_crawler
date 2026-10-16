//! 籌碼資料的 PostgreSQL 倉儲實作。
//!
//! 對應 `chip_daily`、`holder_distribution`、`insider_holding`、`broker_flow` 四張表。
//! 每次寫入都把整批資料展開成陣列參數，以單一 `INSERT ... SELECT FROM UNNEST` 完成；
//! 衝突時只在內容有變才更新（`IS DISTINCT FROM`），重跑不會產生多餘寫入。

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::chip::{
    BrokerFlowRecord, ChipRepository, HolderDistributionRecord, InsiderHoldingRecord,
    InstitutionalNet, MarginRecord,
};

/// 基於 PostgreSQL 的籌碼資料倉儲。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgChipRepository;

impl PgChipRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

mod broker;
mod daily;
mod ownership;

/// 同一批內主鍵重複時 `ON CONFLICT DO UPDATE` 會直接報錯，先以最後一筆為準去重（保留原順序）。
fn dedup_by_key<T, K: std::hash::Hash + Eq>(items: &[T], key: impl Fn(&T) -> K) -> Vec<&T> {
    let mut last: HashMap<K, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        last.insert(key(item), index);
    }
    items
        .iter()
        .enumerate()
        .filter(|(index, item)| last.get(&key(item)) == Some(index))
        .map(|(_, item)| item)
        .collect()
}

#[async_trait]
impl ChipRepository for PgChipRepository {
    async fn save_institutional(&self, date: NaiveDate, flows: &[InstitutionalNet]) -> Result<u64> {
        daily::save_institutional(date, flows).await
    }

    async fn save_margin(&self, date: NaiveDate, margins: &[MarginRecord]) -> Result<u64> {
        daily::save_margin(date, margins).await
    }

    async fn refresh_streaks(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        daily::refresh_streaks(from, to).await
    }

    async fn save_holder_distributions(&self, records: &[HolderDistributionRecord]) -> Result<u64> {
        ownership::save_holder_distributions(records).await
    }

    async fn replace_insider_holdings(&self, records: &[InsiderHoldingRecord]) -> Result<u64> {
        ownership::replace_insider_holdings(records).await
    }

    async fn save_broker_flows(&self, records: &[BrokerFlowRecord]) -> Result<u64> {
        broker::save_broker_flows(records).await
    }
}

/// 測試種入籌碼後重算全部日期的連續天數（測試庫資料量小，整段重算即可）。
#[cfg(test)]
pub(crate) async fn refresh_streaks_for_tests() {
    let from = NaiveDate::from_ymd_opt(1980, 1, 1).expect("固定日期合法");
    let to = NaiveDate::from_ymd_opt(2100, 12, 31).expect("固定日期合法");
    PgChipRepository::new()
        .refresh_streaks(from, to)
        .await
        .expect("重算籌碼連續天數");
}

#[cfg(test)]
mod tests;
