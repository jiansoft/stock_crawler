//! 每週市場摘要的大盤趨勢一段：加權指數收盤、近一週漲跌、均線排列與位置、這週穿越均線的日子、市場廣度、
//! 估值水位（上市櫃本益比、股價淨值比中位數在近 [`valuation::LEVEL_YEARS`] 年的位置）與本月季節性
//! （前 [`SEASONALITY_YEARS`] 年同月的大盤等權 20 日報酬）。

use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Days, NaiveDate};

use super::WEEK_DAYS;
use crate::domain::{
    market_index::{
        breadth::{self, BreadthQuery, MarketBreadth},
        regime::{self, IndexCloseQuery, MarketRegime, MovingAverageCross},
        valuation::{self, MarketValuationRepository, ValuationLevel},
    },
    quote::forward_return::{MarketSeasonalityQuery, MonthSeasonality},
};

/// 本月季節性回看的年數（不含今年：今年這個月的 20 日報酬大多還沒滿期）。
pub(super) const SEASONALITY_YEARS: i32 = 8;

/// 大盤趨勢一段要列出位置的均線天數。
pub(super) const REGIME_MA_DAYS: [u16; 2] = [60, 240];

/// 大盤趨勢一段要列出穿越的均線天數。
pub(super) const REGIME_CROSS_DAYS: [u16; 3] = [20, 60, 240];

/// 大盤趨勢一段：資料日的趨勢狀態、最近 [`WEEK_DAYS`] 個交易日穿越均線的日子（由舊到新）與
/// 市場廣度（沒有統計或讀取失敗為 `None`，不影響其他行）。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RegimeSection {
    pub(super) state: MarketRegime,
    pub(super) crosses: Vec<MovingAverageCross>,
    pub(super) breadth: Option<MarketBreadth>,
    /// 估值水位；由 [`valuation_level`] 另外查詢後填入，沒有資料或讀取失敗為 `None`。
    pub(super) valuation: Option<ValuationLevel>,
    /// 本月季節性；由 [`month_seasonality`] 另外查詢後填入，讀取失敗為 `None`。
    pub(super) seasonality: Option<MonthSeasonality>,
}

/// 資料日所在月份在前 [`SEASONALITY_YEARS`] 年同月（例如 2026-10 取 2018～2025 年的 10 月）的大盤等權
/// 20 日未來報酬。
pub(super) async fn month_seasonality(
    repo: &impl MarketSeasonalityQuery,
    date: NaiveDate,
) -> Result<MonthSeasonality> {
    let (from, to) = seasonality_range(date);
    repo.fetch_month_seasonality(date.month(), from, to).await
}

/// [`month_seasonality`] 的查詢期間：`SEASONALITY_YEARS` 年前同月 1 日到今年同月 1 日的前一天。
pub(super) fn seasonality_range(date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = date.with_day(1).unwrap_or(date);
    let from = first
        .with_year(first.year() - SEASONALITY_YEARS)
        .unwrap_or(NaiveDate::MIN);
    let to = first.checked_sub_days(Days::new(1)).unwrap_or(first);
    (from, to)
}

/// 本月季節性一行，例如「• 10 月季節性（近 8 年同月）：大盤等權 20 日平均報酬 +1.23%、上漲比例 56.0%」；
/// 沒有樣本（平均報酬為 `None`）時回 `None`。
pub(super) fn seasonality_line(month: u32, seasonality: &MonthSeasonality) -> Option<String> {
    let (Some(avg), Some(win)) = (seasonality.avg_return, seasonality.win_rate) else {
        return None;
    };
    Some(format!(
        "• {month} 月季節性（近 {} 年同月）：大盤等權 20 日平均報酬 {avg:+.2}%、上漲比例 {:.1}%",
        seasonality.years,
        win * 100.0
    ))
}

/// `date`（含）之前近 [`valuation::LEVEL_YEARS`] 年的上市櫃估值水位，附 [`WEEK_DAYS`] 個交易日前的本益比
/// 中位數；沒有資料回 `None`。
pub(super) async fn valuation_level(
    repo: &impl MarketValuationRepository,
    date: NaiveDate,
) -> Result<Option<ValuationLevel>> {
    valuation::level(repo, date, WEEK_DAYS).await
}

/// 估值水位一行，例如「• 估值水位 10-16：本益比中位數 17.87 倍（近 5 年第 34 百分位，5 日前 18.10 倍）、
/// 股價淨值比中位數 1.78 倍（第 48 百分位）」；兩者都沒有時回 `None`。
pub(super) fn valuation_line(level: &ValuationLevel) -> Option<String> {
    let summary = level.summary()?;
    Some(format!(
        "• 估值水位 {}：{summary}",
        level.date.format("%m-%d")
    ))
}

/// `date`（含）之前的加權指數趨勢狀態、最近 [`WEEK_DAYS`] 個交易日穿越
/// [`REGIME_CROSS_DAYS`] 均線的日子與同一天的市場廣度；沒有加權指數回 `None`。
pub(super) async fn regime_section(
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
        valuation: None,
        seasonality: None,
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
pub(super) fn lines(section: &RegimeSection) -> Vec<String> {
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
    if let Some(line) = section.valuation.as_ref().and_then(valuation_line) {
        lines.push(line);
    }
    if let Some(line) = section
        .seasonality
        .as_ref()
        .and_then(|seasonality| seasonality_line(state.date.month(), seasonality))
    {
        lines.push(line);
    }
    lines
}
