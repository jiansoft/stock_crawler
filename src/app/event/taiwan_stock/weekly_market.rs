//! # 每週市場摘要
//!
//! 每週六整理最近 [`WEEK_DAYS`] 個交易日的市場輪動，發一則 Telegram：
//!
//! - 大盤趨勢：加權指數收盤、近 [`WEEK_DAYS`] 日漲跌、均線排列、相對 60／240 日均線的位置與方向
//!   （見 [`crate::domain::market_index::regime`]）、這週穿越 20／60／240 日均線的日子，以及市場廣度
//!   （站上 20／60／240 日均線的股票比例、與一週前比較，與這週創 52 週新高／新低的家次，見
//!   [`crate::domain::market_index::breadth`]），以及上市櫃本益比、股價淨值比中位數在近 5 年的位置（見
//!   [`crate::domain::market_index::valuation`]）。
//! - 產業相對強度：成分股 RS 中位數（見 [`crate::domain::quote::relative_strength`]）最強、最弱，
//!   以及與 [`WEEK_DAYS`] 個交易日前相比轉強、轉弱最多的產業；成分股少於
//!   [`industry::MIN_INDUSTRY_STOCKS`] 檔的產業不列。
//! - 近 [`WEEK_DAYS`] 個交易日相對強度站上、跌破 80 與 50 分的家數（每檔每個門檻只取最後一次
//!   穿越），各附分數最高（站上）或最低（跌破）的幾檔。
//! - 外資、投信近 [`WEEK_DAYS`] 個交易日在各產業的估計淨買賣金額（見 [`crate::domain::chip::flow`]），
//!   淨流入與淨流出最多的產業。
//! - 持股（普通股）的 RS 與 [`WEEK_DAYS`] 個交易日前相比上升、下降最多的幾檔。
//! - 持股的基本面警示：營收連續年減、三率三降、營業虧損、自由現金流為負（見 [`crate::domain::portfolio::health`]）。
//! - 有設定策略換股通知的條件時，策略這一期的換股日、本期至今報酬與 0050 對照、最好與最差的持股。
//!
//! 各段各自查詢，某段失敗只記錄警告並略過；都沒有資料時不發送。各段的查詢與文字在子模組
//! [`market`]、[`industry`]、[`crossing`]、[`flows`]、[`holding`]、[`fundamentals`]、[`strategy`]，這裡只負責排程、
//! 查詢順序與組訊息。

mod crossing;
mod flows;
mod fundamentals;
mod holding;
mod industry;
mod market;
mod strategy;

use std::collections::BTreeSet;
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use self::{
    crossing::{CrossingSection, crossing_section},
    flows::{FlowSection, flow_section},
    holding::{HoldingSection, holding_moves},
    industry::{IndustrySection, industry_section},
    market::{RegimeSection, regime_section, valuation_level},
    strategy::{StrategySection, strategy_section},
};
use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        chip::flow::{self, ChipFlowQuery},
        market_index::{
            breadth::BreadthQuery, regime::IndexCloseQuery, valuation::MarketValuationRepository,
        },
        portfolio::health::HoldingFundamentalsQuery,
        quote::{
            forward_return::ForwardReturnRepository,
            relative_strength::{
                RelativeStrengthCrossingQuery, RelativeStrengthQuery, ScoredUniverseQuery,
            },
        },
    },
    infra::database::repository::{
        chip::PgChipRepository, market_index::PgMarketIndexRepository,
        market_valuation::PgMarketValuationRepository, portfolio::PgPortfolioRepository,
        quote::PgQuoteRepository,
    },
};

/// 一週的交易日數。
const WEEK_DAYS: usize = 5;

/// 每一項列出的產業或股票數。
const TOP: usize = 3;

/// 要統計的法人別與名稱。
const FLOW_INVESTORS: [(&str, &str); 2] = [("foreign", "外資"), ("trust", "投信")];

/// 排程入口：彙整本週市場與持股 RS 變化並發送。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch holdings for weekly market summary failed")?;
    let date = Local::now().date_naive();
    let strategy = strategy_section().await;
    let message = run(
        &PgQuoteRepository::new(),
        &PgChipRepository::new(),
        &PgMarketIndexRepository::new(),
        &PgMarketValuationRepository::new(),
        &PgPortfolioRepository::new(),
        date,
        &symbols,
        strategy.as_ref(),
    )
    .await;
    if let Some(message) = message {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 查詢各段資料並組出訊息；某段失敗只記錄警告；都沒有資料時回 `None`。
///
/// 每個資料來源各自傳入，測試才能逐一替換成假倉儲、驗證各段降級。
#[allow(clippy::too_many_arguments)]
async fn run(
    quote: &(
         impl ScoredUniverseQuery
         + RelativeStrengthQuery
         + RelativeStrengthCrossingQuery
         + ForwardReturnRepository
     ),
    chip: &impl ChipFlowQuery,
    index: &(impl IndexCloseQuery + BreadthQuery),
    valuation: &impl MarketValuationRepository,
    portfolio: &impl HoldingFundamentalsQuery,
    date: NaiveDate,
    symbols: &BTreeSet<String>,
    strategy: Option<&StrategySection>,
) -> Option<String> {
    let mut market = regime_section(index, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取加權指數失敗: {why:#}");
        None
    });
    if let Some(section) = market.as_mut() {
        section.valuation = valuation_level(valuation, section.state.date)
            .await
            .unwrap_or_else(|why| {
                tracing::warn!("每週市場摘要：讀取估值水位失敗: {why:#}");
                None
            });
    }
    let industries = industry_section(quote, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取產業相對強度失敗: {why:#}");
        None
    });
    let crossings = crossing_section(quote, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取相對強度穿越失敗: {why:#}");
        None
    });
    let mut flows = Vec::new();
    for (investor, label) in FLOW_INVESTORS {
        match flow::load_flows(chip, date, WEEK_DAYS, investor).await {
            Ok(Some((period, rows))) => flows.push(flow_section(label, period, rows)),
            Ok(None) => {}
            Err(why) => tracing::warn!("每週市場摘要：讀取{label}產業買賣超失敗: {why:#}"),
        }
    }
    let symbols: Vec<String> = symbols.iter().cloned().collect();
    let moves = match quote
        .fetch_relative_strength(date, WEEK_DAYS as u16 + 1, &symbols)
        .await
    {
        Ok(days) => holding_moves(&days),
        Err(why) => {
            tracing::warn!("每週市場摘要：讀取持股相對強度失敗: {why:#}");
            Vec::new()
        }
    };
    let fundamentals = portfolio
        .fetch_holding_fundamentals(&symbols)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("每週市場摘要：讀取持股基本面失敗: {why:#}");
            Vec::new()
        });
    tracing::info!(
        "每週市場摘要: industries={} flows={} holdings={} moves={}",
        industries
            .as_ref()
            .map_or(0, |section| section.industries.len()),
        flows.len(),
        symbols.len(),
        moves.len()
    );
    build_message(
        market.as_ref(),
        industries.as_ref(),
        crossings.as_ref(),
        &flows,
        HoldingSection {
            moves: &moves,
            fundamentals: &fundamentals,
        },
        strategy,
        holdings::stock_name,
    )
}

/// 帶正負號的分數變化文字。
pub(super) fn signed(value: f64) -> String {
    format!("{value:+.1}")
}

/// 億元文字（帶正負號、一位小數）。
pub(super) fn hundred_million(amount: f64) -> String {
    format!("{:+.1} 億", amount / 1e8)
}

/// 組出訊息；各段都沒有內容時回 `None`。
fn build_message(
    market: Option<&RegimeSection>,
    industries: Option<&IndustrySection>,
    crossings: Option<&CrossingSection>,
    flows: &[FlowSection],
    holdings: HoldingSection<'_>,
    strategy: Option<&StrategySection>,
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut data_date = None;

    if let Some(section) = market {
        data_date = Some(section.state.date);
        lines.extend(market::lines(section));
    }

    if let Some(section) = industries.filter(|section| !section.industries.is_empty()) {
        data_date = Some(section.data_date);
        lines.extend(industry::lines(section));
    }

    if let Some(section) = crossings {
        let rows = crossing::lines(section, &stock_name);
        if !rows.is_empty() {
            data_date.get_or_insert(section.to);
            lines.extend(rows);
        }
    }

    for section in flows
        .iter()
        .filter(|section| !section.industries.is_empty())
    {
        data_date.get_or_insert(section.period.data_date);
        lines.extend(flows::lines(section));
    }

    lines.extend(holding::lines(holdings.moves, &stock_name));
    lines.extend(fundamentals::lines(holdings.fundamentals, &stock_name));
    if let Some(section) = strategy {
        lines.extend(strategy::lines(section));
    }

    if lines.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(lines.len() * 96);
    let title = match data_date {
        Some(date) => format!("每週市場摘要︰{date}"),
        None => "每週市場摘要".to_owned(),
    };
    let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(title));
    for line in lines {
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    let _ = writeln!(
        &mut message,
        "\n{}",
        text::escape_markdown_v2("＊金額為每日淨股數×當日收盤價的估計值，過去表現不代表未來結果")
    );
    Some(message)
}

#[cfg(test)]
mod tests;
