//! 報價領域的測試替身（test double）。
//!
//! 只在 `cfg(test)` 下編譯，供各層測試共用，不屬於產品程式碼。
//!
//! 獨立成檔的原因：[`QuoteRepository`] 有十餘個方法，任何測試替身都得把
//! 用不到的方法全部實作成 `unimplemented!()`。這些行永遠不會執行，混在被測
//! 模組的 `mod tests` 裡會被覆蓋率工具算成該模組的未覆蓋行，讓數字失真。
//! 抽到這裡後，覆蓋率設定（`codecov.yml`）可整檔排除。

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::domain::quote::entity::{DailyQuote, LastDailyQuote, QuoteHistoryRecord};
use crate::domain::quote::repository::QuoteRepository;

/// 記錄寫入內容與呼叫次數的假倉儲。
///
/// 用途是驗證流程「有沒有寫、寫了什麼」，以及倉儲失敗時流程如何反應；
/// 未實作的方法一旦被呼叫，`unimplemented!()` 會讓測試直接失敗，
/// 而不是安靜地回傳假資料。
#[derive(Default)]
pub(crate) struct CountingQuoteRepository {
    /// `insert_missing_daily_quotes` 的呼叫次數。
    insert_calls: AtomicUsize,
    /// 歷次收到的報價，依呼叫順序展開存放。
    inserted: Mutex<Vec<DailyQuote>>,
    /// 為 true 時 `insert_missing_daily_quotes` 一律回傳錯誤。
    fail_insert: bool,
}

impl CountingQuoteRepository {
    /// 建立一個「寫入必定失敗」的假倉儲，用於驗證錯誤上拋。
    pub(crate) fn failing() -> Self {
        Self {
            fail_insert: true,
            ..Default::default()
        }
    }

    /// 取得 `insert_missing_daily_quotes` 目前的呼叫次數。
    pub(crate) fn insert_calls(&self) -> usize {
        self.insert_calls.load(Ordering::Relaxed)
    }

    /// 取得目前已收到的報價筆數。
    pub(crate) fn inserted_len(&self) -> usize {
        self.inserted.lock().expect("測試鎖不應中毒").len()
    }

    /// 取得已收到報價的 `(代號, 日期)` 清單，依收到順序排列。
    pub(crate) fn inserted_keys(&self) -> Vec<(String, NaiveDate)> {
        self.inserted
            .lock()
            .expect("測試鎖不應中毒")
            .iter()
            .map(|quote| (quote.stock_symbol.clone(), quote.date))
            .collect()
    }
}

#[async_trait]
impl QuoteRepository for CountingQuoteRepository {
    async fn insert_missing_daily_quotes(&self, quotes: &[DailyQuote]) -> Result<u64> {
        self.insert_calls.fetch_add(1, Ordering::Relaxed);
        if self.fail_insert {
            return Err(anyhow!("假倉儲：寫入失敗"));
        }

        self.inserted
            .lock()
            .expect("測試鎖不應中毒")
            .extend_from_slice(quotes);
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
