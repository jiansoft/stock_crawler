use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::entity::{
    BrokerFlowRecord, HolderDistributionRecord, HolderLevelRecord, InsiderHoldingRecord,
    InstitutionalNet, MarginRecord,
};

/// 最近幾個集保資料週的週摘要。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentHolderWeeks {
    /// 全表的資料日期，由新到舊；`dates[0]` 是最新一週。
    pub dates: Vec<NaiveDate>,
    /// 指定代號在這些日期的週摘要，依代號、日期由新到舊排列；某週沒有資料的代號就沒有那一列。
    pub records: Vec<HolderDistributionRecord>,
}

/// 集保股權分散歷史的讀取合約。
#[async_trait]
pub trait HolderHistoryRepository: Send + Sync {
    /// 讀取 `as_of`（含）以前最近 `weeks` 個資料週，以及 `symbols` 在這幾週的週摘要。
    async fn recent_holder_weeks(
        &self,
        symbols: &[String],
        as_of: NaiveDate,
        weeks: i64,
    ) -> Result<RecentHolderWeeks>;
}

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
