//! 每週市場摘要的相對強度穿越一段：近一週站上、跌破 80 與 50 分的家數與代表股。

use anyhow::Result;
use chrono::NaiveDate;

use super::{TOP, WEEK_DAYS};
use crate::domain::quote::{
    forward_return::ForwardReturnRepository,
    relative_strength::{
        CrossDirection, PeriodCrossing, RelativeStrengthCrossingQuery, THRESHOLDS,
    },
};

/// 一週的相對強度穿越：期間與每檔每個門檻的最後一次穿越。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CrossingSection {
    pub(super) from: NaiveDate,
    pub(super) to: NaiveDate,
    pub(super) crossings: Vec<PeriodCrossing>,
}

/// 最近 [`WEEK_DAYS`] 個交易日的相對強度穿越；沒有交易日回 `None`。
pub(super) async fn crossing_section(
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
pub(super) fn crossing_line(
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

/// 相對強度穿越一段的各行（開頭為空行）；沒有任何穿越時回空陣列。
pub(super) fn lines(
    section: &CrossingSection,
    stock_name: &impl Fn(&str) -> String,
) -> Vec<String> {
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
    if rows.is_empty() {
        return rows;
    }
    let mut lines = vec![String::new()];
    lines.push(format!(
        "相對強度穿越（{}～{}，每檔只取最後一次；後面數字為最新分數）",
        section.from, section.to
    ));
    lines.extend(rows);
    lines
}
