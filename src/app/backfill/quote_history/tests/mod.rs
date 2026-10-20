//! 歷史日線回補的測試。
//!
//! 子模組：`backfill_run`（回補流程）、`month_range`（月份區間）、`market_routing`（依市場分派抓取端）；
//! 這裡放共用的日期建構函式與假抓取端。

mod backfill_run;
mod market_routing;
mod month_range;

use std::collections::HashMap;
use std::sync::Mutex;

use rust_decimal::Decimal;

use super::*;
use crate::domain::quote::test_double::CountingQuoteRepository;
use crate::infra::crawler::share::DailyQuoteDto;

pub(super) fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

/// 依「代號 × 月份」給預先安排好回應的假抓取端。
///
/// 未安排的組合視為該月無資料（回傳空陣列），與 TWSE 查無資料時的行為一致。
#[derive(Default)]
pub(super) struct StubFetcher {
    /// `(代號, 月份)` → 該月要回傳的每日筆數。
    pub(super) responses: HashMap<(String, NaiveDate), usize>,
    /// 一律回傳錯誤的組合，用來模擬單月抓取失敗。
    pub(super) failures: Vec<(String, NaiveDate)>,
    /// 實際被請求過的組合，依呼叫順序記錄。
    pub(super) requested: Mutex<Vec<(String, NaiveDate)>>,
}

impl StubFetcher {
    /// 安排某代號某月回傳 `count` 筆報價。
    pub(super) fn with_quotes(mut self, symbol: &str, month: NaiveDate, count: usize) -> Self {
        self.responses.insert((symbol.to_string(), month), count);
        self
    }

    /// 安排某代號某月抓取失敗。
    pub(super) fn with_failure(mut self, symbol: &str, month: NaiveDate) -> Self {
        self.failures.push((symbol.to_string(), month));
        self
    }

    /// 取得實際請求過的組合。
    pub(super) fn requested(&self) -> Vec<(String, NaiveDate)> {
        self.requested.lock().expect("測試鎖不應中毒").clone()
    }
}

#[async_trait::async_trait]
impl MonthlyQuoteFetcher for StubFetcher {
    async fn fetch(&self, stock_symbol: &str, month: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
        let key = (stock_symbol.to_string(), month);
        self.requested
            .lock()
            .expect("測試鎖不應中毒")
            .push(key.clone());

        if self.failures.contains(&key) {
            anyhow::bail!("假抓取端：{stock_symbol} {} 失敗", month.format("%Y-%m"));
        }

        let count = self.responses.get(&key).copied().unwrap_or(0);
        Ok((0..count)
            .map(|i| {
                let day = u32::try_from(i).expect("測試筆數不會溢位") + 1;
                let quote_date = month.with_day(day).expect("測試日期應合法");
                let mut dto = DailyQuoteDto::new(stock_symbol.to_string(), quote_date);
                dto.closing_price = Decimal::from(10);
                dto
            })
            .collect())
    }
}
