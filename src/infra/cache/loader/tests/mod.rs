//! 全域快取載入與替換的測試。
//!
//! 子模組：`replace_caches`（整批替換與單筆更新）、`loading`（從資料庫載入）。

mod loading;
mod replace_caches;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::market_index::MarketIndex;
use crate::infra::database::table::revenue;

use super::super::share::{SHARE, Share};
