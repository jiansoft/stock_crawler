//! 策略換股通知的測試：策略清單、換股紀錄、通知流程與訊息內容。
//!
//! 子模組：`message`（訊息內容）、`notify`（通知流程與換股紀錄）、`presets`（要通知的策略）；這裡放共用的
//! 日期、假倉儲、假回測閘道與結算建構函式。

mod message;
mod notify;
mod presets;

use std::sync::Mutex;

use anyhow::bail;
use async_trait::async_trait;

use super::message::build_message;
use super::*;
use crate::app::ports::{StrategyHolding, StrategySettlement};
use crate::domain::portfolio::health::{FundamentalSignals, HoldingFundamentals};

pub(super) fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("固定日期合法")
}

pub(in crate::app::event::taiwan_stock) fn holding(
    symbol: &str,
    name: Option<&str>,
    weight: f64,
) -> StrategyHolding {
    StrategyHolding {
        symbol: symbol.to_owned(),
        name: name.map(str::to_owned),
        weight,
        return_to_date: None,
    }
}

pub(in crate::app::event::taiwan_stock) fn rebalance() -> StrategyRebalance {
    StrategyRebalance {
        date: day(16),
        hold_days: 20,
        days_elapsed: 0,
        return_to_date: None,
        etf_return_to_date: None,
        etf_weight: 0,
        market_filtered: false,
        cash: 0.0,
        turnover: 0.25,
        added: vec![holding("2330", Some("台積電"), 50.0)],
        kept: vec![holding("2317", Some("鴻海"), 50.0)],
        exited: vec![holding("79979", None, 50.0)],
    }
}

/// 回傳固定持股基本面的假查詢；`fail` 時查詢失敗。
#[derive(Default)]
pub(super) struct FakeFundamentals {
    pub(super) fundamentals: Vec<HoldingFundamentals>,
    pub(super) fail: bool,
}

impl FakeFundamentals {
    pub(super) fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }
}

#[async_trait]
impl HoldingFundamentalsQuery for FakeFundamentals {
    async fn fetch_holding_fundamentals(
        &self,
        _symbols: &[String],
    ) -> Result<Vec<HoldingFundamentals>> {
        if self.fail {
            bail!("db down");
        }
        Ok(self.fundamentals.clone())
    }
}

/// 記下寫入的換股紀錄。
#[derive(Default)]
pub(super) struct FakeRecorder {
    pub(super) saved: Mutex<Vec<StrategyRebalanceRecord>>,
}

#[async_trait]
impl StrategyRebalanceRepository for FakeRecorder {
    async fn save(&self, record: &StrategyRebalanceRecord) -> Result<bool> {
        self.saved.lock().expect("鎖未中毒").push(record.clone());
        Ok(true)
    }

    async fn list(&self, _preset_name: &str) -> Result<Vec<StrategyRebalanceRecord>> {
        Ok(Vec::new())
    }
}

/// 回傳固定結果並記下收到的條件與結算的策略。
pub(super) struct FakeGateway {
    pub(super) result: Option<StrategyRebalance>,
    pub(super) fail: bool,
    pub(super) settlement: Option<StrategySettlement>,
    pub(super) settlement_fail: bool,
    pub(super) settled: Mutex<Vec<(String, NaiveDate)>>,
    pub(super) queries: Mutex<Vec<String>>,
}

#[async_trait]
impl StrategyGateway for FakeGateway {
    async fn latest_rebalance(&self, query: &str) -> Result<Option<StrategyRebalance>> {
        self.queries
            .lock()
            .expect("鎖未中毒")
            .push(query.to_owned());
        if self.fail {
            bail!("回測條件錯誤");
        }
        Ok(self.result.clone())
    }

    async fn settlement(
        &self,
        preset_name: &str,
        date: NaiveDate,
    ) -> Result<Option<StrategySettlement>> {
        self.settled
            .lock()
            .expect("鎖未中毒")
            .push((preset_name.to_owned(), date));
        if self.settlement_fail {
            bail!("讀不到換股紀錄");
        }
        Ok(self.settlement.clone())
    }
}

pub(super) fn gateway(result: Option<StrategyRebalance>) -> FakeGateway {
    FakeGateway {
        result,
        fail: false,
        settlement: None,
        settlement_fail: false,
        settled: Mutex::new(Vec::new()),
        queries: Mutex::new(Vec::new()),
    }
}

pub(super) fn settlement(etf_weight: u8) -> StrategySettlement {
    StrategySettlement {
        start: NaiveDate::from_ymd_opt(2026, 9, 17).expect("固定日期合法"),
        end: day(16),
        strategy_return: Some(2.034),
        etf_return: Some(-4.21),
        portfolio_return: Some(-1.09),
        etf_weight,
        tracked_from: NaiveDate::from_ymd_opt(2026, 8, 19).expect("固定日期合法"),
        periods: 2,
        cumulative_return: Some(5.5),
        etf_cumulative_return: Some(1.25),
    }
}
