//! 技術指標日線（[`PriceBarRepository`]）與全市場技術訊號（[`TechnicalSignalRepository`]）
//! 的 PostgreSQL 實作。
//!
//! 子模組：[`bars`]（日線、除權息與股數變動、還原日線）、[`signals`]（技術訊號的寫入與查詢、訊號之後的
//! 報酬統計）。

mod bars;
mod signals;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    domain::quote::{
        relative_strength::CROSSING_INDICATORS,
        technical::{
            PriceAdjustment, PriceBar, PriceBarRepository, RankedSignal, RankedSignalQuery,
            SignalOutcome, StockTechnicalSignal, TechnicalSignal, TechnicalSignalQuery,
            TechnicalSignalRepository, VolumeBar, adjust_bars,
        },
    },
    infra::database::{self, repository::quote::PgQuoteRepository},
};

pub(super) use self::signals::insert_signals;

#[cfg(test)]
mod tests;
