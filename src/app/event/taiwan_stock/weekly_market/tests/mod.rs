//! 每週市場摘要的測試：產業 RS 中位數與一週變化、產業法人淨買賣、持股 RS 變化、降級與訊息格式。
//!
//! 子模組：[`fakes`]（假倉儲與固定資料）、[`sections`]（產業、穿越、大盤趨勢、策略各段）、
//! [`message`]（組成整則訊息、各段降級與數字格式）；這裡放共用的資料建構函式。

mod fakes;
mod message;
mod sections;

use std::collections::HashMap;

use anyhow::anyhow;
use async_trait::async_trait;

use super::crossing::crossing_line;
use super::*;
use crate::domain::chip::flow::{FlowPeriod, StockFlow};
use crate::domain::market_index::{breadth::BreadthDay, regime};
use crate::domain::quote::relative_strength::{
    CrossDirection, PeriodCrossing, RelativeStrengthDay, ScoredStock, UniverseRow,
};

pub(super) fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
}

pub(super) fn esc(line: &str) -> String {
    text::escape_markdown_v2(line)
}

pub(super) fn scored(symbol: &str, industry: &str, rs_score: u8) -> ScoredStock {
    ScoredStock {
        rank: 1,
        rs_score,
        weighted: f64::from(rs_score),
        row: UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            market_id: 2,
            industry: industry.to_owned(),
            return_20d: None,
            return_60d: 0.0,
            return_120d: 0.0,
            return_240d: 0.0,
            closing_price: None,
        },
    }
}

/// 同一產業的多檔股票，代號以產業名加序號。
pub(super) fn industry(name: &str, scores: &[u8]) -> Vec<ScoredStock> {
    scores
        .iter()
        .enumerate()
        .map(|(index, score)| scored(&format!("{name}{index}"), name, *score))
        .collect()
}

/// universe 101 時分數＝weaker × 98 ÷ 100 取整＋1。
pub(super) fn rs_day(symbol: &str, d: u32, weaker: i64) -> RelativeStrengthDay {
    RelativeStrengthDay {
        stock_symbol: symbol.to_owned(),
        date: day(d),
        weighted: 0.0,
        rs_score: crate::domain::quote::relative_strength::percentile_score(
            usize::try_from(weaker).expect("測試資料非負"),
            101,
        ),
        rank: 101 - weaker,
        universe: 101,
    }
}

pub(super) fn flow(symbol: &str, industry: &str, amount: f64) -> StockFlow {
    StockFlow {
        stock_symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        market_id: 2,
        industry: industry.to_owned(),
        net_shares: 0,
        net_amount: amount,
        volume: None,
        trade_value: None,
        buy_days: 0,
        sell_days: 0,
        streak: 0,
        closing_price: None,
    }
}
