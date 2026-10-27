use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::entity::{
    BrokerFlowRecord, HolderDistributionRecord, HolderLevelRecord, InsiderHoldingRecord,
    InstitutionalNet, MarginRecord,
};

/// 籌碼資料的倉儲合約。各方法回傳實際新增或變更的列數，內容相同的重送不算。
#[async_trait]
pub trait ChipRepository: Send + Sync {
    /// 寫入（或更正）指定交易日的三大法人買賣超；不動同一列的融資融券欄位。
    async fn save_institutional(&self, date: NaiveDate, flows: &[InstitutionalNet]) -> Result<u64>;

    /// 寫入（或更正）指定交易日的融資融券餘額；不動同一列的三大法人欄位。
    async fn save_margin(&self, date: NaiveDate, margins: &[MarginRecord]) -> Result<u64>;

    /// 重算 `from`～`to`（皆含）每列的外資、投信、自營商與合計連續買賣超天數。
    ///
    /// 依日期遞推：方向與同一檔前一列相同就累加，淨額 0 或缺值為 0；期間第一段接續 `from` 之前
    /// 最後一列的天數。補寫或更正過去的籌碼後，要從那天起重算到最新一天，後面的天數才會正確。
    async fn refresh_streaks(&self, from: NaiveDate, to: NaiveDate) -> Result<u64>;

    /// 寫入（或更正）集保股權分散週摘要。
    async fn save_holder_distributions(&self, records: &[HolderDistributionRecord]) -> Result<u64>;

    /// 寫入（或更正）集保股權分散的全部持股分級。
    async fn save_holder_levels(&self, records: &[HolderLevelRecord]) -> Result<u64>;

    /// 以這批資料取代同月份、同股票的董監持股（內部人異動時舊名單整批換掉）。
    async fn replace_insider_holdings(&self, records: &[InsiderHoldingRecord]) -> Result<u64>;

    /// 寫入（或更正）券商分點主力進出。
    async fn save_broker_flows(&self, records: &[BrokerFlowRecord]) -> Result<u64>;
}
