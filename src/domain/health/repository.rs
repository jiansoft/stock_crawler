use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::entity::DataHealthSnapshot;

/// 資料健康檢查的倉儲合約。
#[async_trait]
pub trait DataHealthRepository: Send + Sync {
    /// 量測 `from`～`to`（皆含）的資料狀況。
    ///
    /// `baseline_from` 是成交檔數比較基準的起始日（早於 `from`），
    /// `current_year` 用來判斷「尚未公布」的股利佔位列是否已過期。
    async fn fetch_snapshot(
        &self,
        baseline_from: NaiveDate,
        from: NaiveDate,
        to: NaiveDate,
        current_year: i32,
    ) -> Result<DataHealthSnapshot>;
}
