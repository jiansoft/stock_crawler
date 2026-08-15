//! 回補流程對外部資料來源的需求介面（port）。
//!
//! 這裡宣告的是「app 層需要什麼」，實作由 infra 的 crawler 提供
//! （依賴方向 infra → app）。抽這一層的目的只有一個：讓編排流程本身
//! 可以在不連外部網站的情況下被驗證 —— 例如「單一月份抓取失敗要略過並繼續」
//! 這種承諾，唯有替換掉抓取端才測得到。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::infra::crawler::share::DailyQuoteDto;

/// 以「單一股票 × 單一月份」為單位取得每日成交資訊。
#[async_trait]
pub trait MonthlyQuoteFetcher: Send + Sync {
    /// 取得 `stock_symbol` 在 `month` 所屬月份的每日報價。
    ///
    /// `month` 只取年月。查無資料（例如該股當月尚未上市）回傳空陣列而非錯誤；
    /// 只有真正的失敗（網路、來源格式異動）才回傳 `Err`。
    async fn fetch(&self, stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>>;
}
