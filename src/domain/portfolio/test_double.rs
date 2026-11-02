//! 測試用的持股倉儲替身：只實作股利入帳查詢，記錄每次查詢的期間並回傳預先設定的分配。

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::entity::{ReceivedDividend, ReceivedDividendItem, StockOwnershipDetail};
use super::payout::PayoutAllocation;
use super::repository::PortfolioRepository;

/// 股利入帳查詢的倉儲替身；其他方法不會被呼叫。
#[derive(Debug, Default)]
pub(crate) struct FakePayoutRepository {
    /// 每次查詢都回傳的分配（不依期間過濾，交給 `split_by_payable_date`）。
    pub(crate) allocations: Vec<PayoutAllocation>,
    /// 每次查詢的 `(member_id, from, to, today)`。
    pub(crate) queries: Mutex<Vec<(i64, NaiveDate, NaiveDate, NaiveDate)>>,
    /// 為 `true` 時查詢回傳錯誤。
    pub(crate) fail: bool,
}

impl FakePayoutRepository {
    /// 每次查詢都回傳 `allocations`。
    pub(crate) fn with_allocations(allocations: Vec<PayoutAllocation>) -> Self {
        Self {
            allocations,
            ..Self::default()
        }
    }

    /// 目前為止的查詢紀錄。
    pub(crate) fn queries(&self) -> Vec<(i64, NaiveDate, NaiveDate, NaiveDate)> {
        self.queries.lock().expect("queries").clone()
    }
}

#[async_trait]
impl PortfolioRepository for FakePayoutRepository {
    async fn fetch_active_holdings(
        &self,
        _security_codes: Option<Vec<String>>,
    ) -> Result<Vec<StockOwnershipDetail>> {
        unimplemented!("入帳測試不查持股明細")
    }

    async fn find_holding_by_serial(&self, _serial: i64) -> Result<Option<StockOwnershipDetail>> {
        unimplemented!("入帳測試不查持股明細")
    }

    async fn update_holding_dividends(&self, _holding: &StockOwnershipDetail) -> Result<()> {
        unimplemented!("入帳測試不寫入")
    }

    async fn save_received_dividend(
        &self,
        _summary: &ReceivedDividend,
        _items: &[ReceivedDividendItem],
    ) -> Result<()> {
        unimplemented!("入帳測試不寫入")
    }

    async fn delete_received_dividend(&self, _holding_serial: i64, _year: i32) -> Result<()> {
        unimplemented!("入帳測試不寫入")
    }

    async fn calculate_accumulated_dividends(
        &self,
        _holding_serial: i64,
    ) -> Result<(Decimal, Decimal, Decimal, Decimal)> {
        unimplemented!("入帳測試不查累積股利")
    }

    async fn fetch_payout_allocations(
        &self,
        member_id: i64,
        from: NaiveDate,
        to: NaiveDate,
        today: NaiveDate,
    ) -> Result<Vec<PayoutAllocation>> {
        self.queries
            .lock()
            .expect("queries")
            .push((member_id, from, to, today));
        if self.fail {
            anyhow::bail!("入帳查詢測試失敗");
        }
        Ok(self.allocations.clone())
    }
}
