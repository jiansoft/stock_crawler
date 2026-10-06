use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::entity::{
    BrokerFlowRecord, HolderDistributionRecord, InsiderHoldingRecord, InstitutionalNet,
    MarginRecord,
};

/// 籌碼資料的倉儲合約。各方法回傳實際新增或變更的列數，內容相同的重送不算。
#[async_trait]
pub trait ChipRepository: Send + Sync {
    /// 寫入（或更正）指定交易日的三大法人買賣超；不動同一列的融資融券欄位。
    async fn save_institutional(&self, date: NaiveDate, flows: &[InstitutionalNet]) -> Result<u64>;

    /// 寫入（或更正）指定交易日的融資融券餘額；不動同一列的三大法人欄位。
    async fn save_margin(&self, date: NaiveDate, margins: &[MarginRecord]) -> Result<u64>;

    /// 寫入（或更正）集保股權分散週摘要。
    async fn save_holder_distributions(&self, records: &[HolderDistributionRecord]) -> Result<u64>;

    /// 以這批資料取代同月份、同股票的董監持股（內部人異動時舊名單整批換掉）。
    async fn replace_insider_holdings(&self, records: &[InsiderHoldingRecord]) -> Result<u64>;

    /// 寫入（或更正）券商分點主力進出。
    async fn save_broker_flows(&self, records: &[BrokerFlowRecord]) -> Result<u64>;
}
