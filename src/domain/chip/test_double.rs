//! 測試用的籌碼倉儲替身：記錄每次寫入的方法與筆數，可設定讓寫入失敗。

use std::sync::Mutex;

use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::NaiveDate;

use super::entity::{
    BrokerFlowRecord, HolderDistributionRecord, HolderLevelRecord, InsiderHoldingRecord,
    InstitutionalNet, MarginRecord,
};
use super::repository::ChipRepository;

/// 記錄寫入的籌碼倉儲；每次呼叫記成一行「方法 參數」，`refresh_streaks` 固定回 5 列。
#[derive(Debug, Default)]
pub(crate) struct RecordingChipRepository {
    /// 歷次呼叫，例如 `institutional 2026-10-02 3`。
    calls: Mutex<Vec<String>>,
    /// 為 `true` 時所有寫入都回錯誤（仍會記錄呼叫）。
    fail: bool,
}

impl RecordingChipRepository {
    /// 所有寫入都失敗的替身。
    pub(crate) fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    /// 歷次呼叫。
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("鎖未中毒").clone()
    }

    fn record(&self, call: String, rows: usize) -> Result<u64> {
        self.calls.lock().expect("鎖未中毒").push(call);
        if self.fail {
            bail!("測試寫入失敗");
        }
        Ok(rows as u64)
    }
}

#[async_trait]
impl ChipRepository for RecordingChipRepository {
    async fn save_institutional(&self, date: NaiveDate, flows: &[InstitutionalNet]) -> Result<u64> {
        self.record(format!("institutional {date} {}", flows.len()), flows.len())
    }

    async fn save_margin(&self, date: NaiveDate, margins: &[MarginRecord]) -> Result<u64> {
        self.record(format!("margin {date} {}", margins.len()), margins.len())
    }

    async fn refresh_streaks(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        self.record(format!("streaks {from} {to}"), 5)
    }

    async fn save_holder_distributions(&self, records: &[HolderDistributionRecord]) -> Result<u64> {
        self.record(
            format!("holder_distributions {}", records.len()),
            records.len(),
        )
    }

    async fn save_holder_levels(&self, records: &[HolderLevelRecord]) -> Result<u64> {
        self.record(format!("holder_levels {}", records.len()), records.len())
    }

    async fn replace_insider_holdings(&self, records: &[InsiderHoldingRecord]) -> Result<u64> {
        self.record(format!("insider {}", records.len()), records.len())
    }

    async fn save_broker_flows(&self, records: &[BrokerFlowRecord]) -> Result<u64> {
        self.record(format!("broker_flows {}", records.len()), records.len())
    }
}
