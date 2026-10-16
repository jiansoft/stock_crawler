//! # 大盤趨勢變化通知
//!
//! 每個交易日 15:30（15:00 收盤事件寫入加權指數之後）檢查加權指數當天是否站上或跌破
//! [`ALERT_MA_DAYS`] 日均線、均線排列是否改變（見 [`regime`]），有變化才發 Telegram。
//! 最新一筆加權指數不是今天（休市或尚未寫入）就不發。
//!
//! 條件選股回測的大盤濾網（`market_filter_ma`）也是看加權指數相對均線的位置，這則通知讓
//! 環境轉換當天就知道。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use crate::{
    core::{alert, util::text},
    domain::market_index::regime::{self, IndexCloseQuery, MarketRegime},
    infra::database::repository::market_index::PgMarketIndexRepository,
};

/// 要通知穿越的均線天數。
const ALERT_MA_DAYS: [u16; 2] = [60, 240];

/// 排程入口：今天加權指數有穿越均線或排列改變就發通知。
///
/// # Errors
///
/// 讀取加權指數失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let today = Local::now().date_naive();
    if let Some(message) = run(&PgMarketIndexRepository::new(), today).await? {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 讀取加權指數並組出訊息；最新一筆不是 `today` 或沒有變化時回 `None`。
async fn run(index: &impl IndexCloseQuery, today: NaiveDate) -> Result<Option<String>> {
    let points = regime::load_taiex(index, Some(today))
        .await
        .context("load TAIEX closes for market regime alert failed")?;
    let latest = points.first().map(|(date, _)| *date);
    if latest != Some(today) {
        tracing::info!("大盤趨勢變化：最新加權指數為 {latest:?}，不是今天 {today}，略過");
        return Ok(None);
    }
    Ok(build_message(&points))
}

/// 由新到舊的收盤組出訊息：最新一天穿越 [`ALERT_MA_DAYS`] 日均線、或排列和前一天不同；
/// 都沒有時回 `None`。
fn build_message(points: &[(NaiveDate, f64)]) -> Option<String> {
    let today = regime::regime(points)?;
    let previous = regime::regime(points.get(1..)?);
    let crosses: Vec<_> = ALERT_MA_DAYS
        .iter()
        .flat_map(|&days| regime::crosses(points, days, 1))
        .collect();
    let alignment_change = previous
        .as_ref()
        .filter(|previous| previous.alignment != today.alignment);
    if crosses.is_empty() && alignment_change.is_none() {
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
