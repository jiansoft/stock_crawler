//! CAGR 計算流程的單元測試（以假資料來源 `FakeSource` 取代資料庫）。
//!
//! 子模組：[`fakes`]（假資料來源與記錄寫入的假倉儲）、[`calculate`]（每日計算流程）、
//! [`backfill`]（指定期間回補與純函式）；這裡放共用的日期與數值建構函式。

mod backfill;
mod calculate;
mod fakes;

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::domain::performance::StockCagr;
use crate::domain::performance::entity::{CagrCoverage, CagrMetric};
use crate::domain::performance::query::{CagrRankingPage, CagrRankingQuery};

pub(super) fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

pub(super) fn dec(value: i64) -> Decimal {
    Decimal::from(value)
}
