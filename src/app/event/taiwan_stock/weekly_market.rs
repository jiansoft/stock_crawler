//! # 每週市場摘要
//!
//! 每週六整理最近 [`WEEK_DAYS`] 個交易日的市場輪動，發一則 Telegram：
//!
//! - 產業相對強度：成分股 RS 中位數（見 [`crate::domain::quote::relative_strength`]）最強、最弱，
//!   以及與 [`WEEK_DAYS`] 個交易日前相比轉強、轉弱最多的產業；成分股少於 [`MIN_INDUSTRY_STOCKS`]
//!   檔的產業不列。
//! - 外資、投信近 [`WEEK_DAYS`] 個交易日在各產業的估計淨買賣金額（見 [`crate::domain::chip::flow`]），
//!   淨流入與淨流出最多的產業。
//! - 持股（普通股）的 RS 與 [`WEEK_DAYS`] 個交易日前相比上升、下降最多的幾檔。
//!
//! 三段各自查詢，某段失敗只記錄警告並略過；三段都沒有資料時不發送。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        chip::flow::{self, ChipFlowQuery, FlowPeriod, StockFlow},
        quote::{
            forward_return::ForwardReturnRepository,
            relative_strength::{
                RelativeStrengthDay, RelativeStrengthQuery, ScoredUniverseQuery, group_by_industry,
                median_score,
            },
        },
    },
    infra::database::repository::{chip::PgChipRepository, quote::PgQuoteRepository},
};

/// 一週的交易日數。
const WEEK_DAYS: usize = 5;

/// 產業至少要有幾檔計分股票才列入（檔數太少的中位數不穩定）。
const MIN_INDUSTRY_STOCKS: usize = 5;

/// 每一項列出的產業或股票數。
const TOP: usize = 3;

/// 要統計的法人別與名稱。
const FLOW_INVESTORS: [(&str, &str); 2] = [("foreign", "外資"), ("trust", "投信")];

/// 產業 RS 中位數與一週變化。
#[derive(Debug, Clone, PartialEq)]
struct IndustryScore {
    industry: String,
    median: f64,
    /// 與一週前中位數的差；一週前沒有該產業時為 `None`。
    change: Option<f64>,
}

/// 產業 RS 一段：資料日與各產業，由強到弱。
#[derive(Debug, Clone, PartialEq)]
struct IndustrySection {
    data_date: NaiveDate,
    industries: Vec<IndustryScore>,
}

/// 單一法人的產業淨買賣一段：期間與各產業淨額（元），由多到少。
#[derive(Debug, Clone, PartialEq)]
struct FlowSection {
    label: &'static str,
    period: FlowPeriod,
    industries: Vec<(String, f64)>,
}

/// 一檔持股的 RS 一週變化。
#[derive(Debug, Clone, PartialEq)]
struct HoldingMove {
    stock_symbol: String,
    from: u8,
    to: u8,
}

impl HoldingMove {
    fn change(&self) -> i16 {
        i16::from(self.to) - i16::from(self.from)
    }
}

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
    let message = run(
        &PgQuoteRepository::new(),
        &PgChipRepository::new(),
        date,
        &symbols,
    )
    .await;
    if let Some(message) = message {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 查詢三段資料並組出訊息；某段失敗只記錄警告；都沒有資料時回 `None`。
async fn run(
    quote: &(impl ScoredUniverseQuery + RelativeStrengthQuery + ForwardReturnRepository),
    chip: &impl ChipFlowQuery,
    date: NaiveDate,
    symbols: &BTreeSet<String>,
) -> Option<String> {
    let industries = industry_section(quote, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取產業相對強度失敗: {why:#}");
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
    tracing::info!(
        "每週市場摘要: industries={} flows={} holdings={} moves={}",
        industries
            .as_ref()
            .map_or(0, |section| section.industries.len()),
        flows.len(),
        symbols.len(),
        moves.len()
    );
    build_message(industries.as_ref(), &flows, &moves, holdings::stock_name)
}

/// 最新資料日與 [`WEEK_DAYS`] 個交易日前的產業 RS 中位數；沒有分數回 `None`。
async fn industry_section(
    quote: &(impl ScoredUniverseQuery + ForwardReturnRepository),
    date: NaiveDate,
) -> Result<Option<IndustrySection>> {
    let Some((data_date, current)) = quote.fetch_scored_universe(date).await? else {
        return Ok(None);
    };
    let dates = quote.fetch_trading_dates(data_date, WEEK_DAYS + 1).await?;
    let prior = match dates.get(WEEK_DAYS) {
        Some(&prior_until) => quote.fetch_scored_universe(prior_until).await?,
        None => None,
    };
    let prior_medians: BTreeMap<String, f64> = prior
        .map(|(_, scored)| {
            group_by_industry(scored, 0)
                .into_iter()
                .filter_map(|(industry, stocks)| Some((industry, median_score(&stocks)?)))
                .collect()
        })
        .unwrap_or_default();
    let mut industries: Vec<IndustryScore> = group_by_industry(current, 0)
        .into_iter()
        .filter(|(_, stocks)| stocks.len() >= MIN_INDUSTRY_STOCKS)
        .filter_map(|(industry, stocks)| {
            let median = median_score(&stocks)?;
            Some(IndustryScore {
                change: prior_medians.get(&industry).map(|prior| median - prior),
                industry,
                median,
            })
        })
        .collect();
    industries.sort_by(|a, b| {
        b.median
            .total_cmp(&a.median)
            .then_with(|| a.industry.cmp(&b.industry))
    });
    Ok(Some(IndustrySection {
        data_date,
        industries,
    }))
}

/// 依產業加總估計淨買賣金額（未分類略過），由多到少。
fn flow_section(label: &'static str, period: FlowPeriod, rows: Vec<StockFlow>) -> FlowSection {
    let mut totals: BTreeMap<String, f64> = BTreeMap::new();
    for row in rows {
        if !row.industry.is_empty() {
            *totals.entry(row.industry).or_default() += row.net_amount;
        }
    }
    let mut industries: Vec<(String, f64)> = totals.into_iter().collect();
    industries.sort_by(|(a_name, a), (b_name, b)| b.total_cmp(a).then_with(|| a_name.cmp(b_name)));
    FlowSection {
        label,
        period,
        industries,
    }
}

/// 由依代號、再由新到舊排列的分數算出每檔持股最新分數與最舊一筆的差；只有一筆的略過。
fn holding_moves(days: &[RelativeStrengthDay]) -> Vec<HoldingMove> {
    days.chunk_by(|a, b| a.stock_symbol == b.stock_symbol)
        .filter_map(|group| {
            let (latest, oldest) = (group.first()?, group.last()?);
            (group.len() > 1).then(|| HoldingMove {
                stock_symbol: latest.stock_symbol.clone(),
                from: oldest.rs_score,
                to: latest.rs_score,
            })
        })
        .collect()
}

/// 帶正負號的分數變化文字。
fn signed(value: f64) -> String {
    format!("{value:+.1}")
}

/// 億元文字（帶正負號、一位小數）。
fn hundred_million(amount: f64) -> String {
    format!("{:+.1} 億", amount / 1e8)
}

/// 組出訊息；三段都沒有內容時回 `None`。
fn build_message(
    industries: Option<&IndustrySection>,
    flows: &[FlowSection],
    moves: &[HoldingMove],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut data_date = None;

    if let Some(section) = industries.filter(|section| !section.industries.is_empty()) {
        data_date = Some(section.data_date);
        let list = |items: &mut dyn Iterator<Item = &IndustryScore>, with_change: bool| {
            items
                .take(TOP)
                .map(|item| match (with_change, item.change) {
                    (true, Some(change)) => {
                        format!("{} {}（{}）", item.industry, item.median, signed(change))
                    }
                    _ => format!("{} {}", item.industry, item.median),
                })
                .collect::<Vec<_>>()
                .join("、")
        };
        lines.push(String::new());
        lines.push(format!(
            "產業相對強度（成分股 RS 中位數，括號為與 {WEEK_DAYS} 個交易日前相比）"
        ));
        lines.push(format!(
            "• 最強：{}",
            list(&mut section.industries.iter(), true)
        ));
        lines.push(format!(
            "• 最弱：{}",
            list(&mut section.industries.iter().rev(), true)
        ));
        let mut changed: Vec<(&str, f64)> = section
            .industries
            .iter()
            .filter_map(|item| Some((item.industry.as_str(), item.change?)))
            .collect();
        changed.sort_by(|(a_name, a), (b_name, b)| b.total_cmp(a).then_with(|| a_name.cmp(b_name)));
        let movers = |items: &mut dyn Iterator<Item = &(&str, f64)>| {
            items
                .take(TOP)
                .map(|(industry, change)| format!("{industry} {}", signed(*change)))
                .collect::<Vec<_>>()
                .join("、")
        };
        let up = movers(&mut changed.iter().filter(|(_, change)| *change > 0.0));
        if !up.is_empty() {
            lines.push(format!("• 轉強：{up}"));
        }
        let down = movers(&mut changed.iter().rev().filter(|(_, change)| *change < 0.0));
        if !down.is_empty() {
            lines.push(format!("• 轉弱：{down}"));
        }
    }

    for section in flows
        .iter()
        .filter(|section| !section.industries.is_empty())
    {
        data_date.get_or_insert(section.period.data_date);
        let list = |items: &mut dyn Iterator<Item = &(String, f64)>| {
            items
                .take(TOP)
                .map(|(industry, amount)| format!("{industry} {}", hundred_million(*amount)))
                .collect::<Vec<_>>()
                .join("、")
        };
        lines.push(String::new());
        lines.push(format!(
            "{}近 {} 日產業淨買賣（{}～{}）",
            section.label, section.period.days, section.period.from, section.period.data_date
        ));
        let inflow = list(
            &mut section
                .industries
                .iter()
                .filter(|(_, amount)| *amount > 0.0),
        );
        lines.push(format!(
            "• 流入：{}",
            if inflow.is_empty() { "無" } else { &inflow }
        ));
        let outflow = list(
            &mut section
                .industries
                .iter()
                .rev()
                .filter(|(_, amount)| *amount < 0.0),
        );
        lines.push(format!(
            "• 流出：{}",
            if outflow.is_empty() { "無" } else { &outflow }
        ));
    }

    let mut sorted: Vec<&HoldingMove> = moves.iter().filter(|item| item.change() != 0).collect();
    sorted.sort_by(|a, b| {
        b.change()
            .cmp(&a.change())
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    if !sorted.is_empty() {
        let line = |item: &&HoldingMove| {
            format!(
                "{} {} {} → {}（{:+}）",
                item.stock_symbol,
                stock_name(&item.stock_symbol),
                item.from,
                item.to,
                item.change()
            )
        };
        lines.push(String::new());
        lines.push(format!("持股相對強度（與 {WEEK_DAYS} 個交易日前相比）"));
        let up: Vec<String> = sorted
            .iter()
            .filter(|item| item.change() > 0)
            .take(TOP)
            .map(line)
            .collect();
        if !up.is_empty() {
            lines.push(format!("• 上升：{}", up.join("、")));
        }
        let down: Vec<String> = sorted
            .iter()
            .rev()
            .filter(|item| item.change() < 0)
            .take(TOP)
            .map(line)
            .collect();
        if !down.is_empty() {
            lines.push(format!("• 下降：{}", down.join("、")));
        }
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
