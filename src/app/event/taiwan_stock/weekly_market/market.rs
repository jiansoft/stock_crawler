//! 每週市場摘要的大盤趨勢一段：加權指數收盤、近一週漲跌、均線排列與位置、這週穿越均線的日子與市場廣度。

use std::fmt::Write;

use anyhow::Result;
use chrono::NaiveDate;

use super::WEEK_DAYS;
use crate::domain::market_index::{
    breadth::{self, BreadthQuery, MarketBreadth},
    regime::{self, IndexCloseQuery, MarketRegime, MovingAverageCross},
};

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
    lines
}
