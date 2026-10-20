//! 每日 CAGR 倉儲（`stock_cagr`）：寫入（[`persist`]）、排行榜（[`ranking`]）與其他查詢（[`lookup`]）
//! 分在子模組，這裡放倉儲本體、共用的欄位清單與 trait 實作的轉呼叫。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::performance::entity::{
    CagrCoverage, CagrMetric, CagrPeriod, StockCagr as DomainStockCagr,
};
use crate::domain::performance::query::{CagrRankingPage, CagrRankingQuery};
use crate::domain::performance::repository::CagrRepository;
use crate::infra::database::table::performance::stock_cagr::StockCagr as TableStockCagr;

mod lookup;
mod persist;
mod ranking;

/// 讀取用的欄位清單。
///
/// 與 [`TableStockCagr`] 的欄位順序一致；集中於此避免各查詢各寫一份而失去同步。
const SELECT_COLUMNS: &str = r#"
    date, stock_symbol, period, base_date, base_price, end_price, years,
    price_end_shares, price_end_value, price_return_pct, price_cagr_pct,
    total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct,
    reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct,
    first_quote_date, shortfall_days, data_complete, has_anomaly, dividend_events
"#;

/// 基於 PostgreSQL 的每日 CAGR 倉儲實現 (PgCagrRepository)。
///
/// 對應資料表 `public.stock_cagr`，主鍵為 `(date, stock_symbol, period)`。
pub struct PgCagrRepository;

impl PgCagrRepository {
    /// 建立新的 PgCagrRepository 實例。
    pub fn new() -> Self {
        PgCagrRepository
    }
}

impl Default for PgCagrRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// 依報酬口徑取得對應的年化報酬率欄位名稱。
///
/// 回傳值是編譯期字面值，**不是**呼叫端傳入的字串——排序鍵必須以白名單
/// 映射，任何把外部輸入拼進 SQL 的寫法都是注入破口。
fn cagr_column(metric: CagrMetric) -> &'static str {
    match metric {
        CagrMetric::Price => "price_cagr_pct",
        CagrMetric::Total => "total_cagr_pct",
        CagrMetric::Reinvested => "reinv_cagr_pct",
    }
}

/// 將資料列批次還原為領域實體，並依期間長度由短至長排序。
fn rows_to_domain(rows: Vec<TableStockCagr>) -> Result<Vec<DomainStockCagr>> {
    rows.into_iter().map(|row| row.to_domain()).collect()
}

#[async_trait]
impl CagrRepository for PgCagrRepository {
    /// 批次 upsert 指定基準日的 CAGR 計算結果，見 [`persist::save_batch`]。
    async fn save_batch(&self, records: &[DomainStockCagr]) -> Result<u64> {
        persist::save_batch(records).await
    }

    /// 取得指定基準日與期間的排行榜，見 [`ranking::fetch_ranking`]。
    async fn fetch_ranking(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DomainStockCagr>> {
        ranking::fetch_ranking(date, period, metric, limit, offset).await
    }

    /// 依完整查詢條件取得排行榜單頁結果，見 [`ranking::fetch_ranking_page`]。
    async fn fetch_ranking_page(&self, query: &CagrRankingQuery) -> Result<CagrRankingPage> {
        ranking::fetch_ranking_page(query).await
    }

    /// 取得單一個股在指定基準日的所有期間結果，見 [`lookup::fetch_by_symbol`]。
    async fn fetch_by_symbol(
        &self,
        date: NaiveDate,
        stock_symbol: &str,
    ) -> Result<Vec<DomainStockCagr>> {
        lookup::fetch_by_symbol(date, stock_symbol).await
    }

    /// 取得指定基準日與期間的樣本涵蓋統計，見 [`lookup::fetch_coverage`]。
    async fn fetch_coverage(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
    ) -> Result<CagrCoverage> {
        lookup::fetch_coverage(date, period, metric).await
    }

    /// 取得最新一個已完成計算的基準日。
    async fn fetch_latest_date(&self) -> Result<Option<NaiveDate>> {
        lookup::fetch_latest_date().await
    }

    async fn fetch_dates_missing_period(&self, period: CagrPeriod) -> Result<Vec<NaiveDate>> {
        lookup::fetch_dates_missing_period(period).await
    }

    /// 刪除早於指定日期的歷史資料。
    async fn delete_before(&self, date: NaiveDate) -> Result<u64> {
        persist::delete_before(date).await
    }
}

#[cfg(test)]
mod tests;
