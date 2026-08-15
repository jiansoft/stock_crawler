//! 報價領域的測試替身（test double）。
//!
//! 只在 `cfg(test)` 下編譯，供各層測試共用，不屬於產品程式碼。
//!
//! 獨立成檔的原因：[`QuoteRepository`] 有十餘個方法，任何測試替身都得把
//! 用不到的方法全部實作成 `unimplemented!()`。這些行永遠不會執行，混在被測
//! 模組的 `mod tests` 裡會被覆蓋率工具算成該模組的未覆蓋行，讓數字失真。
//! 抽到這裡後，覆蓋率設定（`codecov.yml`）可整檔排除。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::domain::quote::entity::{DailyQuote, LastDailyQuote, QuoteHistoryRecord};
use crate::domain::quote::repository::QuoteRepository;

/// 只記錄「被呼叫過幾次」的假倉儲。
///
/// 用途是證明流程在不該寫入時完全沒有碰倉儲；未實作的方法一旦被呼叫，
/// `unimplemented!()` 會讓測試直接失敗，而不是安靜地回傳假資料。
#[derive(Default)]
pub(crate) struct CountingQuoteRepository {
    /// `insert_missing_daily_quotes` 的呼叫次數。
    insert_calls: AtomicUsize,
}

impl CountingQuoteRepository {
    /// 取得 `insert_missing_daily_quotes` 目前的呼叫次數。
    pub(crate) fn insert_calls(&self) -> usize {
        self.insert_calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl QuoteRepository for CountingQuoteRepository {
    async fn insert_missing_daily_quotes(&self, quotes: &[DailyQuote]) -> Result<u64> {
        self.insert_calls.fetch_add(1, Ordering::Relaxed);
        Ok(quotes.len() as u64)
    }

    async fn save_daily_quote(&self, _quote: &DailyQuote) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn batch_save_daily_quotes(&self, _quotes: &[DailyQuote]) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn fetch_quotes_by_date(&self, _date: NaiveDate) -> Result<Vec<DailyQuote>> {
        unimplemented!("測試不應走到這裡")
    }

    async fn delete_quotes_by_date(&self, _date: NaiveDate) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn replace_quotes_by_date(
        &self,
        _date: NaiveDate,
        _quotes: &[DailyQuote],
    ) -> Result<u64> {
        unimplemented!("測試不應走到這裡")
    }

    async fn fill_moving_average(&self, _quote: &mut DailyQuote) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn batch_update_moving_average(&self, _quotes: &[DailyQuote]) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn makeup_for_the_lack_daily_quotes(&self, _date: NaiveDate) -> Result<u64> {
        unimplemented!("測試不應走到這裡")
    }

    async fn fetch_monthly_stock_price_summary(
        &self,
        _security_code: &str,
        _year: i32,
        _month: i32,
    ) -> Result<Option<(Decimal, Decimal, Decimal)>> {
        unimplemented!("測試不應走到這裡")
    }

    async fn fetch_last_daily_quotes(&self) -> Result<Vec<LastDailyQuote>> {
        unimplemented!("測試不應走到這裡")
    }

    async fn rebuild_last_daily_quotes(&self) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn fetch_last_quote(&self, _security_code: &str) -> Result<Option<LastDailyQuote>> {
        unimplemented!("測試不應走到這裡")
    }

    async fn save_last_quotes_batch(&self, _quotes: &[LastDailyQuote]) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn save_stock_price_stats(&self, _date: NaiveDate) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }

    async fn fetch_quote_history_records(&self) -> Result<Vec<QuoteHistoryRecord>> {
        unimplemented!("測試不應走到這裡")
    }

    async fn save_quote_history_record(&self, _record: &QuoteHistoryRecord) -> Result<()> {
        unimplemented!("測試不應走到這裡")
    }
}
