use crate::domain::quote::entity::{DailyQuote, LastDailyQuote, QuoteHistoryRecord};
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 報價領域之倉儲介面 (Repository Trait)。
///
/// 隔離資料庫與 Redis 存取細節，定義對日報價、最新交易日報價與技術估值統計的讀寫合約。
#[async_trait]
pub trait QuoteRepository: Send + Sync {
    // === 每日報價 (DailyQuote) ===

    /// 儲存或更新單筆日報價。
    async fn save_daily_quote(&self, quote: &DailyQuote) -> Result<()>;

    /// 批次儲存或更新多筆日報價（適用於資料庫快速匯入）。
    async fn batch_save_daily_quotes(&self, quotes: &[DailyQuote]) -> Result<()>;

    /// 依交易日查詢全市場的每日報價資料。
    async fn fetch_quotes_by_date(&self, date: NaiveDate) -> Result<Vec<DailyQuote>>;

    /// 刪除指定交易日的所有每日報價。
    async fn delete_quotes_by_date(&self, date: NaiveDate) -> Result<()>;

    /// 依指定日期與股票，查詢並回填該股票的均線與年內高低點統計。
    async fn fill_moving_average(&self, quote: &mut DailyQuote) -> Result<()>;

    /// 批次更新每日收盤均線、年內統計與 PBR。
    async fn batch_update_moving_average(&self, quotes: &[DailyQuote]) -> Result<()>;

    /// 補上當日缺少的每日收盤數據。
    ///
    /// 回傳受影響的資料筆數。
    async fn makeup_for_the_lack_daily_quotes(&self, date: NaiveDate) -> Result<u64>;

    /// 取得指定股票在指定年月的最低、平均、最高收盤價統計。
    ///
    /// 回傳格式為：`(最低價, 平均價, 最高價)`。
    async fn fetch_monthly_stock_price_summary(
        &self,
        security_code: &str,
        year: i32,
        month: i32,
    ) -> Result<Option<(Decimal, Decimal, Decimal)>>;

    // === 最新報價 (LastDailyQuote) ===

    /// 取得所有個股在最後交易日的最新收盤價資料。
    async fn fetch_last_daily_quotes(&self) -> Result<Vec<LastDailyQuote>>;

    /// 重建最新交易日報價表（例如以近 30 天內最新數據填補）。
    async fn rebuild_last_daily_quotes(&self) -> Result<()>;

    /// 取得指定個股的最新價格狀態。
    ///
    /// 此方法應在實作中封裝 Cache-Aside 快取策略（優先查詢 Redis 快取，若未命中則查詢 PostgreSQL 並自動回寫快取）。
    async fn fetch_last_quote(&self, security_code: &str) -> Result<Option<LastDailyQuote>>;

    /// 批次儲存或更新最新個股收盤價。
    async fn save_last_quotes_batch(&self, quotes: &[LastDailyQuote]) -> Result<()>;

    // === 股價分布統計 (DailyStockPriceStats) ===

    /// 產生或更新指定日期的股價分布統計資料。
    async fn save_stock_price_stats(&self, date: NaiveDate) -> Result<()>;

    // === 歷史極值紀錄 (QuoteHistoryRecord) ===

    /// 取得所有個股的歷史價格與股價淨值比極值紀錄。
    async fn fetch_quote_history_records(&self) -> Result<Vec<QuoteHistoryRecord>>;

    /// 新增或更新單一股票的歷史極值資料。
    async fn save_quote_history_record(&self, record: &QuoteHistoryRecord) -> Result<()>;
}
