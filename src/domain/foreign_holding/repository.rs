use anyhow::Result;
use async_trait::async_trait;

use crate::domain::foreign_holding::entity::{ForeignHolding, HoldingForeignHoldingAlert};

/// 外資持股歷史與趨勢之倉儲介面。
#[async_trait]
pub trait ForeignHoldingRepository: Send + Sync {
    /// 寫入（或更正）每日外資持股快照，回傳實際新增或變更的筆數。
    ///
    /// 同一 `(股票代號, 日期)` 重送視為更正；內容相同時不更新。
    async fn save_daily(&self, holdings: &[ForeignHolding]) -> Result<u64>;

    /// 以歷史資料的最新交易日為基準，整批重算所有股票的外資持股趨勢，回傳寫入筆數。
    async fn rebuild_trends(&self) -> Result<u64>;

    /// 取得目前持股（未賣出）的外資持股趨勢。
    async fn fetch_holding_alerts(&self) -> Result<Vec<HoldingForeignHoldingAlert>>;
}
