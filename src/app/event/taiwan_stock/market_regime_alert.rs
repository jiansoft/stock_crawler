//! # 大盤趨勢變化通知
//!
//! 每個交易日 15:30（15:00 收盤事件寫入加權指數之後）檢查加權指數當天是否站上或跌破
//! [`ALERT_MA_DAYS`] 日均線、均線排列是否改變（見 [`regime`]），以及上市櫃站上 60 日均線的
//! 股票比例是否跨過 [`ALERT_BREADTH_PERCENT`]（見 [`breadth`]），有變化才發 Telegram；發送時一併
//! 附上當天的市場廣度與估值水位（[`valuation::level`]；官方本益比 17:30 才補齊，通知裡的估值多半是前一個
//! 交易日，行內標出日期）。最新一筆加權指數不是今天（休市或尚未寫入）就不發；市場廣度或估值讀取失敗、
//! 沒有資料時只略過那一行。
//!
//! 條件選股回測的大盤濾網（`market_filter_ma`）與市場廣度濾網（`market_filter_breadth`）也是看
//! 這兩件事，這則通知讓環境轉換當天就知道。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use crate::{
    core::{alert, util::text},
    domain::market_index::{
        breadth::{self, BreadthDay, BreadthQuery},
        regime::{self, IndexCloseQuery, MarketRegime},
        valuation::{self, MarketValuationRepository, ValuationLevel},
    },
    infra::database::repository::{
        market_index::PgMarketIndexRepository, market_valuation::PgMarketValuationRepository,
    },
};

/// 估值水位附帶幾個交易日前的本益比中位數。
const VALUATION_BEFORE_DAYS: usize = 5;

/// 要通知穿越的均線天數。
const ALERT_MA_DAYS: [u16; 2] = [60, 240];

/// 站上 60 日均線的股票比例（%）跨過這條線就通知；回測時 60 日均線濾網再加上市場廣度 30% 的
/// 濾網效果最好，與 `market_filter_breadth` 一樣以「不低於」算通過。
const ALERT_BREADTH_PERCENT: f64 = 30.0;

/// 排程入口：今天加權指數有穿越均線或排列改變就發通知。
///
/// # Errors
///
/// 讀取加權指數失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let today = Local::now().date_naive();
    if let Some(message) = run(
        &PgMarketIndexRepository::new(),
        &PgMarketValuationRepository::new(),
        today,
    )
    .await?
    {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 讀取加權指數、市場廣度與估值水位並組出訊息；最新一筆不是 `today` 或沒有變化時回 `None`。
async fn run(
    index: &(impl IndexCloseQuery + BreadthQuery),
    valuation_repo: &impl MarketValuationRepository,
    today: NaiveDate,
) -> Result<Option<String>> {
    let points = regime::load_taiex(index, Some(today))
        .await
        .context("load TAIEX closes for market regime alert failed")?;
    let latest = points.first().map(|(date, _)| *date);
    if latest != Some(today) {
        tracing::info!("大盤趨勢變化：最新加權指數為 {latest:?}，不是今天 {today}，略過");
        return Ok(None);
    }
    let breadth = match index.fetch_breadth(Some(today), 2).await {
        Ok(days) if days.first().is_some_and(|day| day.date == today) => days,
        Ok(_) => Vec::new(),
        Err(error) => {
            tracing::warn!("大盤趨勢變化：讀取市場廣度失敗，略過廣度：{error:?}");
            Vec::new()
        }
    };
    let level = valuation::level(valuation_repo, today, VALUATION_BEFORE_DAYS)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("大盤趨勢變化：讀取估值水位失敗，略過估值：{why:#}");
            None
        });
    Ok(build_message(&points, &breadth, level.as_ref()))
}

/// 由新到舊的收盤與市場廣度組出訊息：最新一天穿越 [`ALERT_MA_DAYS`] 日均線、排列和前一天不同、
/// 或站上 60 日均線的比例跨過 [`ALERT_BREADTH_PERCENT`]；都沒有時回 `None`。`breadth` 為今天與
/// 前一個交易日的統計（由新到舊），沒有時不看廣度；`valuation` 只附在訊息裡，不影響是否發送。
fn build_message(
    points: &[(NaiveDate, f64)],
    breadth: &[BreadthDay],
    valuation: Option<&ValuationLevel>,
) -> Option<String> {
    let today = regime::regime(points)?;
    let previous = regime::regime(points.get(1..)?);
    let crosses: Vec<_> = ALERT_MA_DAYS
        .iter()
        .flat_map(|&days| regime::crosses(points, days, 1))
        .collect();
    let alignment_change = previous
        .as_ref()
        .filter(|previous| previous.alignment != today.alignment);
    let breadth = breadth_change(breadth);
    let breadth_crossed = breadth.is_some_and(|(now, before)| {
        (now >= ALERT_BREADTH_PERCENT) != (before >= ALERT_BREADTH_PERCENT)
    });
    if crosses.is_empty() && alignment_change.is_none() && !breadth_crossed {
        return None;
    }

    let mut lines = vec![format!(
        "加權指數收盤 {:.2}（{}）",
        today.close,
        today
            .change_percent
            .map_or_else(|| "—".to_owned(), |value| format!("{value:+.2}%"))
    )];
    for cross in &crosses {
        lines.extend(cross_line(
            &today,
            previous.as_ref(),
            cross.days,
            cross.above,
        ));
    }
    if let Some(previous) = alignment_change {
        lines.push(format!(
            "• 均線排列：{} → {}",
            previous.alignment.label(),
            today.alignment.label()
        ));
    }
    if let Some((now, before)) = breadth {
        lines.push(breadth_line(now, before));
    }
    if let Some(level) = valuation
        && let Some(summary) = level.summary()
    {
        lines.push(format!(
            "• 估值水位 {}：{summary}",
            level.date.format("%m-%d")
        ));
    }

    let mut message = String::with_capacity(256);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("大盤趨勢變化︰{}", today.date))
    );
    for line in lines {
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    let _ = writeln!(
        &mut message,
        "\n{}",
        text::escape_markdown_v2("＊只描述價格與均線的位置，不代表後續走勢")
    );
    Some(message)
}

/// 今天與前一個交易日站上 60 日均線的股票比例（%）；任一天沒有統計為 `None`。
fn breadth_change(days: &[BreadthDay]) -> Option<(f64, f64)> {
    let summary = breadth::summarize(days, 1)?;
    Some((
        summary.above_ma60_percent?,
        summary.above_ma60_percent_before?,
    ))
}

/// 市場廣度的一行：今天與前一日的比例，跨過 [`ALERT_BREADTH_PERCENT`] 時註明方向。
fn breadth_line(now: f64, before: f64) -> String {
    let mut line = format!("• 市場廣度：站上 60 日均線的股票 {now:.2}%（前一日 {before:.2}%）");
    let threshold = ALERT_BREADTH_PERCENT;
    match (now >= threshold, before >= threshold) {
        (true, false) => {
            let _ = write!(&mut line, "，回到 {threshold:.0}% 以上");
        }
        (false, true) => {
            let _ = write!(&mut line, "，跌破 {threshold:.0}%");
        }
        _ => {}
    }
    line
}

/// 穿越 `days` 日均線的一行；60 日均線另附穿越前連續在另一側的交易日數。
fn cross_line(
    today: &MarketRegime,
    previous: Option<&MarketRegime>,
    days: u16,
    above: bool,
) -> Option<String> {
    let status = today.moving_average(days)?;
    let (value, distance) = (status.value?, status.distance_percent?);
    let (action, side, other_side) = if above {
        ("站上", "上方", "下方")
    } else {
        ("跌破", "下方", "上方")
    };
    let mut line = format!(
        "• {action} {days} 日均線 {value:.2}（收盤在{side} {:.2}%）",
        distance.abs()
    );
    if days == 60
        && let Some(count) = previous.and_then(|previous| previous.days_on_side_of_ma60)
    {
        let _ = write!(&mut line, "，之前連續 {count} 個交易日在{other_side}");
    }
    Some(line)
}

#[cfg(test)]
mod tests;
