use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::performance::entity::{CagrCoverage, CagrMetric, CagrPeriod, StockCagr};

/// 績效指標領域之倉儲介面 (Repository Trait)。
///
/// 隔離 PostgreSQL 存取細節，定義每日 CAGR 計算結果的讀寫合約。
#[async_trait]
pub trait CagrRepository: Send + Sync {
    /// 批次寫入或更新指定基準日的 CAGR 計算結果。
    ///
    /// 同一 `(date, stock_symbol, period)` 重複執行必須是冪等的
    /// （以 upsert 覆蓋），讓同日重跑可以修正先前的結果。
    ///
    /// 回傳實際寫入的資料筆數。
    async fn save_batch(&self, records: &[StockCagr]) -> Result<u64>;

    /// 取得指定基準日與期間的排行榜，依指定口徑的年化報酬率由高至低排序。
    ///
    /// 僅回傳 `data_complete = true` 的資料；資料不足者由呼叫端另行查詢。
    async fn fetch_ranking(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StockCagr>>;

    /// 取得單一個股在指定基準日的所有期間結果（含資料不足者）。
    async fn fetch_by_symbol(&self, date: NaiveDate, stock_symbol: &str)
    -> Result<Vec<StockCagr>>;

    /// 取得指定基準日與期間的樣本涵蓋統計。
    async fn fetch_coverage(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
    ) -> Result<CagrCoverage>;

    /// 取得最新一個已完成計算的基準日。尚無資料時回傳 `None`。
    async fn fetch_latest_date(&self) -> Result<Option<NaiveDate>>;

    /// 刪除早於指定日期的歷史資料，回傳刪除筆數。
    async fn delete_before(&self, date: NaiveDate) -> Result<u64>;
}
