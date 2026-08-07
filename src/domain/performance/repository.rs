use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::performance::entity::{CagrCoverage, CagrMetric, CagrPeriod, StockCagr};
use crate::domain::performance::query::{CagrRankingPage, CagrRankingQuery};

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
    /// 指定口徑的 `*_cagr_pct` 為 NULL 者一併排除 —— 長期間不提供純價格口徑
    /// （見 [`CagrPeriod::supports_price_metric`]），會出現 `data_complete = true`
    /// 但該口徑為 NULL 的列。
    async fn fetch_ranking(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StockCagr>>;

    /// 依完整查詢條件取得排行榜單頁結果（含篩選、總筆數與樣本涵蓋統計）。
    ///
    /// 與 [`Self::fetch_ranking`] 的差別在於這是給 API 用的完整查詢：支援市場、
    /// 產業、關鍵字篩選，回傳股票名稱與名次，並一併帶出分頁所需的總筆數。
    ///
    /// 實作要求：
    ///
    /// - `include_incomplete` 為 `true` 時，資料不足的項目要一併回傳，
    ///   但 `rank` 為 `None` 且**排在所有可算項目之後**。
    /// - 排序鍵與排序欄位必須以白名單對應到字面值，不得字串拼接。
    async fn fetch_ranking_page(&self, query: &CagrRankingQuery) -> Result<CagrRankingPage>;

    /// 取得單一個股在指定基準日的所有期間結果（含資料不足者）。
    ///
    /// 結果必須依**期間長度**（[`CagrPeriod::months`]）由短至長排序。注意不能
    /// 用資料庫端的 `period` 字串排序 —— 字典序會把 `Y10` 排在 `Y1H` 之前。
    async fn fetch_by_symbol(&self, date: NaiveDate, stock_symbol: &str) -> Result<Vec<StockCagr>>;

    /// 取得指定基準日與期間的樣本涵蓋統計。
    ///
    /// `universe` 取自該 `(date, period)` 的資料列數，因此**計算層必須為母體中
    /// 每一檔股票都寫入一列**（資料不足者寫 `data_complete = false` 的 NULL 列）。
    /// 若略過不寫，`universe` 會退化成「有寫入的檔數」，涵蓋率被系統性高估 ——
    /// 而涵蓋率正是 Y5／Y10 揭露存活者偏誤的依據，高估等於掩蓋問題。
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
