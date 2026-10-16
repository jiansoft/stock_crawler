//! # 每週市場摘要
//!
//! 每週六整理最近 [`WEEK_DAYS`] 個交易日的市場輪動，發一則 Telegram：
//!
//! - 大盤趨勢：加權指數收盤、近 [`WEEK_DAYS`] 日漲跌、均線排列、相對 60／240 日均線的位置與方向
//!   （見 [`crate::domain::market_index::regime`]）、這週穿越 20／60／240 日均線的日子，以及市場廣度
//!   （站上 20／60／240 日均線的股票比例、與一週前比較，與這週創 52 週新高／新低的家次，見
//!   [`crate::domain::market_index::breadth`]）。
//! - 產業相對強度：成分股 RS 中位數（見 [`crate::domain::quote::relative_strength`]）最強、最弱，
//!   以及與 [`WEEK_DAYS`] 個交易日前相比轉強、轉弱最多的產業；成分股少於 [`MIN_INDUSTRY_STOCKS`]
//!   檔的產業不列。
//! - 近 [`WEEK_DAYS`] 個交易日相對強度站上、跌破 80 與 50 分的家數（每檔每個門檻只取最後一次
//!   穿越），各附分數最高（站上）或最低（跌破）的幾檔。
//! - 外資、投信近 [`WEEK_DAYS`] 個交易日在各產業的估計淨買賣金額（見 [`crate::domain::chip::flow`]），
//!   淨流入與淨流出最多的產業。
//! - 持股（普通股）的 RS 與 [`WEEK_DAYS`] 個交易日前相比上升、下降最多的幾檔。
//!
//! 各段各自查詢，某段失敗只記錄警告並略過；都沒有資料時不發送。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::{
        chip::flow::{self, ChipFlowQuery, FlowPeriod, StockFlow},
        market_index::{
            breadth::{self, BreadthQuery, MarketBreadth},
            regime::{self, IndexCloseQuery, MarketRegime, MovingAverageCross},
        },
        quote::{
            forward_return::ForwardReturnRepository,
            relative_strength::{
                CrossDirection, PeriodCrossing, RelativeStrengthCrossingQuery, RelativeStrengthDay,
                RelativeStrengthQuery, ScoredUniverseQuery, THRESHOLDS, group_by_industry,
                median_score,
            },
        },
    },
    infra::database::repository::{
        chip::PgChipRepository, market_index::PgMarketIndexRepository, quote::PgQuoteRepository,
    },
};

/// 一週的交易日數。
const WEEK_DAYS: usize = 5;

/// 產業至少要有幾檔計分股票才列入（檔數太少的中位數不穩定）。
const MIN_INDUSTRY_STOCKS: usize = 5;

/// 每一項列出的產業或股票數。
const TOP: usize = 3;

/// 要統計的法人別與名稱。
const FLOW_INVESTORS: [(&str, &str); 2] = [("foreign", "外資"), ("trust", "投信")];

/// 大盤趨勢一段要列出位置的均線天數。
const REGIME_MA_DAYS: [u16; 2] = [60, 240];

/// 大盤趨勢一段要列出穿越的均線天數。
const REGIME_CROSS_DAYS: [u16; 3] = [20, 60, 240];

/// 大盤趨勢一段：資料日的趨勢狀態、最近 [`WEEK_DAYS`] 個交易日穿越均線的日子（由舊到新）與
/// 市場廣度（沒有統計或讀取失敗為 `None`，不影響其他行）。
#[derive(Debug, Clone, PartialEq)]
struct RegimeSection {
    state: MarketRegime,
    crosses: Vec<MovingAverageCross>,
    breadth: Option<MarketBreadth>,
}

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

/// 一週的相對強度穿越：期間與每檔每個門檻的最後一次穿越。
#[derive(Debug, Clone, PartialEq)]
struct CrossingSection {
    from: NaiveDate,
    to: NaiveDate,
    crossings: Vec<PeriodCrossing>,
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
        &PgMarketIndexRepository::new(),
        date,
        &symbols,
    )
    .await;
    if let Some(message) = message {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 查詢各段資料並組出訊息；某段失敗只記錄警告；都沒有資料時回 `None`。
async fn run(
    quote: &(
         impl ScoredUniverseQuery
         + RelativeStrengthQuery
         + RelativeStrengthCrossingQuery
         + ForwardReturnRepository
     ),
    chip: &impl ChipFlowQuery,
    index: &(impl IndexCloseQuery + BreadthQuery),
    date: NaiveDate,
    symbols: &BTreeSet<String>,
) -> Option<String> {
    let market = regime_section(index, date).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取加權指數失敗: {why:#}");
        None
    });
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
        &moves,
        holdings::stock_name,
    )
}

/// `date`（含）之前的加權指數趨勢狀態、最近 [`WEEK_DAYS`] 個交易日穿越
/// [`REGIME_CROSS_DAYS`] 均線的日子與同一天的市場廣度；沒有加權指數回 `None`。
async fn regime_section(
    index: &(impl IndexCloseQuery + BreadthQuery),
    date: NaiveDate,
) -> Result<Option<RegimeSection>> {
    let points = regime::load_taiex(index, Some(date)).await?;
    let Some(state) = regime::regime(&points) else {
        return Ok(None);
    };
    let mut crosses: Vec<MovingAverageCross> = REGIME_CROSS_DAYS
        .iter()
        .flat_map(|&days| regime::crosses(&points, days, WEEK_DAYS))
        .collect();
    crosses.sort_by_key(|cross| (cross.date, cross.days));
    let breadth = breadth::load(index, Some(state.date), WEEK_DAYS)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("每週市場摘要：讀取市場廣度失敗: {why:#}");
            None
        });
    Ok(Some(RegimeSection {
        state,
        crosses,
        breadth,
    }))
}

/// 市場廣度一行，例如「• 市場廣度 10-16：站上 60 日線 40.0%（5 日前 65.0%）、20 日線 30.0%、
/// 240 日線 —；近 5 日創 52 週新高 15 家次、新低 15 家次」。
fn breadth_line(breadth: &MarketBreadth) -> String {
    let percent = |value: Option<f64>| value.map_or_else(|| "—".to_owned(), |v| format!("{v:.1}%"));
    let before = breadth
        .above_ma60_percent_before
        .map_or_else(String::new, |value| {
            format!("（{WEEK_DAYS} 日前 {value:.1}%）")
        });
    format!(
        "• 市場廣度 {}：站上 60 日線 {}{before}、20 日線 {}、240 日線 {}；近 {} 日創 52 週新高 {} 家次、新低 {} 家次",
        breadth.date.format("%m-%d"),
        percent(breadth.above_ma60_percent),
        percent(breadth.above_ma20_percent),
        percent(breadth.above_ma240_percent),
        breadth.days,
        breadth.new_highs,
        breadth.new_lows
    )
}

/// 大盤趨勢一段的各行。
fn regime_lines(section: &RegimeSection) -> Vec<String> {
    let state = &section.state;
    let mut lines = vec![
        String::new(),
        format!("大盤趨勢（加權指數 {}）", state.date),
        format!(
            "• 收盤 {:.2}，近 {WEEK_DAYS} 日 {}；{}",
            state.close,
            state
                .return_5d
                .map_or_else(|| "—".to_owned(), |value| format!("{value:+.2}%")),
            state.alignment.label()
        ),
    ];
    for days in REGIME_MA_DAYS {
        let Some(status) = state.moving_average(days) else {
            continue;
        };
        let (Some(value), Some(distance)) = (status.value, status.distance_percent) else {
            lines.push(format!("• {days} 日均線資料不足"));
            continue;
        };
        let side = if distance >= 0.0 { "上方" } else { "下方" };
        let trend = if status.rising == Some(true) {
            "上揚"
        } else {
            "下彎"
        };
        let mut line = format!(
            "• {days} 日均線 {value:.2}，收盤在{side} {:.2}%（均線{trend}）",
            distance.abs()
        );
        if days == 60
            && let Some(count) = state.days_on_side_of_ma60
        {
            let _ = write!(&mut line, "，連續 {count} 個交易日在{side}");
        }
        lines.push(line);
    }
    let crosses: Vec<String> = section
        .crosses
        .iter()
        .map(|cross| {
            format!(
                "{} {} {} 日均線",
                cross.date.format("%m-%d"),
                if cross.above { "站上" } else { "跌破" },
                cross.days
            )
        })
        .collect();
    let days = REGIME_CROSS_DAYS.map(|days| days.to_string()).join("／");
    lines.push(if crosses.is_empty() {
        format!("• 近 {WEEK_DAYS} 日沒有穿越 {days} 日均線")
    } else {
        format!("• 近 {WEEK_DAYS} 日：{}", crosses.join("、"))
    });
    if let Some(breadth) = &section.breadth {
        lines.push(breadth_line(breadth));
    }
    lines
}

/// 最近 [`WEEK_DAYS`] 個交易日的相對強度穿越；沒有交易日回 `None`。
async fn crossing_section(
    quote: &(impl RelativeStrengthCrossingQuery + ForwardReturnRepository),
    date: NaiveDate,
) -> Result<Option<CrossingSection>> {
    let dates = quote.fetch_trading_dates(date, WEEK_DAYS).await?;
    let (Some(&to), Some(&from)) = (dates.first(), dates.last()) else {
        return Ok(None);
    };
    let crossings = quote.fetch_period_crossings(from, to).await?;
    Ok(Some(CrossingSection {
        from,
        to,
        crossings,
    }))
}

/// 穿越一行，例如「站上 80 分 45 檔：2330 台積電 92、…」；沒有股票時回 `None`。站上依最新分數
/// 由高到低、跌破由低到高列前 [`TOP`] 檔（沒有最新分數的排最後）。
fn crossing_line(
    crossings: &[PeriodCrossing],
    threshold: u8,
    direction: CrossDirection,
    stock_name: &impl Fn(&str) -> String,
) -> Option<String> {
    let mut picked: Vec<&PeriodCrossing> = crossings
        .iter()
        .filter(|crossing| crossing.threshold == threshold && crossing.direction == direction)
        .collect();
    if picked.is_empty() {
        return None;
    }
    picked.sort_by_key(|crossing| {
        let score = crossing.latest_score.map(i16::from);
        let key = match direction {
            CrossDirection::Up => score.map(|score| -score),
            CrossDirection::Down => score,
        };
        (key.is_none(), key, crossing.stock_symbol.clone())
    });
    let examples: Vec<String> = picked
        .iter()
        .take(TOP)
        .map(|crossing| {
            let name = stock_name(&crossing.stock_symbol);
            match crossing.latest_score {
                Some(score) => format!("{} {name} {score}", crossing.stock_symbol),
                None => format!("{} {name}", crossing.stock_symbol),
            }
        })
        .collect();
    let verb = match direction {
        CrossDirection::Up => "站上",
        CrossDirection::Down => "跌破",
    };
    Some(format!(
        "• {verb} {threshold} 分 {} 檔：{}",
        picked.len(),
        examples.join("、")
    ))
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

/// 組出訊息；各段都沒有內容時回 `None`。
fn build_message(
    market: Option<&RegimeSection>,
    industries: Option<&IndustrySection>,
    crossings: Option<&CrossingSection>,
    flows: &[FlowSection],
    moves: &[HoldingMove],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut data_date = None;

    if let Some(section) = market {
        data_date = Some(section.state.date);
        lines.extend(regime_lines(section));
    }

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

    if let Some(section) = crossings {
        let rows: Vec<String> = THRESHOLDS
            .into_iter()
            .flat_map(|threshold| {
                [
                    (threshold, CrossDirection::Up),
                    (threshold, CrossDirection::Down),
                ]
            })
            .filter_map(|(threshold, direction)| {
                crossing_line(&section.crossings, threshold, direction, &stock_name)
            })
            .collect();
        if !rows.is_empty() {
            data_date.get_or_insert(section.to);
            lines.push(String::new());
            lines.push(format!(
                "相對強度穿越（{}～{}，每檔只取最後一次；後面數字為最新分數）",
                section.from, section.to
            ));
            lines.extend(rows);
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
