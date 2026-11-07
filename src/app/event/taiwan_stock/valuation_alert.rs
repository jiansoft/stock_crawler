//! # 持股估值週報
//!
//! 每週六把持股的本益比、股價淨值比放回自己近 5 年的歷史裡比較，列出目前位於高檔
//! （≥ 第 90 百分位）或低檔（≤ 第 10 百分位）的持股。統計規則與 Data API 的
//! `/stocks/{symbol}/valuation-band` 相同（[`valuation_band::summarize`]）。
//!
//! - 只看持股中的普通股（[`holdings::common_stock_holdings`]）。
//! - 有效樣本不足 [`MIN_SAMPLES`] 天（新上市、長期虧損）或最新報價超過
//!   [`MAX_STALE_DAYS`] 天（停牌）的股票不列入，避免樣本太少的百分位誤導。
//! - 沒有任何持股落在高檔或低檔時不發送。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::quote::valuation_band::{self, DailyRatio, ValuationHistoryRepository},
    infra::database::repository::quote::PgQuoteRepository,
};

/// 回看幾年。
const YEARS: u8 = 5;

/// 百分位 ≥ 這個值算高檔。
const HIGH_PERCENTILE: f64 = 90.0;

/// 百分位 ≤ 這個值算低檔。
const LOW_PERCENTILE: f64 = 10.0;

/// 有效樣本至少幾天（約一年的交易日）。
const MIN_SAMPLES: usize = 250;

/// 最新報價距今超過幾天就不列入。
const MAX_STALE_DAYS: i64 = 10;

/// 估值倍數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ratio {
    PriceEarning,
    PriceToBook,
}

impl Ratio {
    fn label(self) -> &'static str {
        match self {
            Self::PriceEarning => "本益比",
            Self::PriceToBook => "股價淨值比",
        }
    }

    fn pick(self, row: &DailyRatio) -> Decimal {
        match self {
            Self::PriceEarning => row.price_earning_ratio,
            Self::PriceToBook => row.price_to_book_ratio,
        }
    }
}

/// 落在高檔或低檔的一個倍數。
#[derive(Debug, Clone, PartialEq)]
struct Flag {
    stock_symbol: String,
    ratio: Ratio,
    current: f64,
    percentile: f64,
    median: f64,
    high: bool,
}

/// 排程入口：檢查持股估值並在有高檔或低檔持股時通知。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤（交給排程告警）；單檔查詢失敗只記錄、不中斷。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for valuation alert failed")?;
    let flags = collect_flags(
        &PgQuoteRepository::new(),
        symbols.iter().map(String::as_str),
        Local::now().date_naive(),
    )
    .await;
    tracing::info!(
        "持股估值週報: holdings={} flagged={}",
        symbols.len(),
        flags.len()
    );
    if let Some(message) = build_message(&flags, holdings::stock_name) {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 逐檔取每日倍數並找出高檔與低檔；單檔失敗只記錄。
async fn collect_flags<'a>(
    repo: &impl ValuationHistoryRepository,
    symbols: impl IntoIterator<Item = &'a str>,
    today: NaiveDate,
) -> Vec<Flag> {
    let mut flags = Vec::new();
    for symbol in symbols {
        match repo.fetch_daily_ratios(symbol, YEARS).await {
            Ok(rows) => flags.extend(evaluate(symbol, &rows, today)),
            Err(why) => tracing::warn!("持股估值週報查詢 {symbol} 失敗: {why:#}"),
        }
    }
    flags
}

/// 單檔的兩個倍數各自判斷；`rows` 依日期由新到舊。
fn evaluate(symbol: &str, rows: &[DailyRatio], today: NaiveDate) -> Vec<Flag> {
    let Some(latest) = rows.first() else {
        return Vec::new();
    };
    if (today - latest.date).num_days() > MAX_STALE_DAYS {
        return Vec::new();
    }
    [Ratio::PriceEarning, Ratio::PriceToBook]
        .into_iter()
        .filter_map(|ratio| {
            let history: Vec<Decimal> = rows.iter().map(|row| ratio.pick(row)).collect();
            let band =
                valuation_band::summarize(&history, Some(ratio.pick(latest)), Some(latest.close))?;
            let (current, percentile) = band.current.zip(band.percentile)?;
            let high = percentile >= HIGH_PERCENTILE;
            (band.samples >= MIN_SAMPLES && (high || percentile <= LOW_PERCENTILE)).then(|| Flag {
                stock_symbol: symbol.to_owned(),
                ratio,
                current,
                percentile,
                median: band.median,
                high,
            })
        })
        .collect()
}

/// 組出週報（Telegram MarkdownV2）；沒有任何高檔或低檔時回 `None`。
fn build_message(flags: &[Flag], stock_name: impl Fn(&str) -> String) -> Option<String> {
    if flags.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(256 + flags.len() * 96);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持股估值週報（近 {YEARS} 年本益比／股價淨值比）"))
    );
    for (high, heading) in [
        (true, format!("偏高（≥ 第 {HIGH_PERCENTILE} 百分位）")),
        (false, format!("偏低（≤ 第 {LOW_PERCENTILE} 百分位）")),
    ] {
        let mut group: Vec<&Flag> = flags.iter().filter(|flag| flag.high == high).collect();
        if group.is_empty() {
            continue;
        }
        // 偏高由最高排起、偏低由最低排起。
        group.sort_by(|a, b| {
            let order = a.percentile.total_cmp(&b.percentile);
            if high { order.reverse() } else { order }
        });
        let _ = write!(&mut message, "\n{}\n", text::escape_markdown_v2(heading));
        for flag in group {
            let line = format!(
                "{} {} {} {} 倍，第 {} 百分位（中位數 {} 倍）",
                flag.stock_symbol,
                stock_name(&flag.stock_symbol),
                flag.ratio.label(),
                round2(flag.current),
                flag.percentile,
                round2(flag.median)
            );
            let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
        }
    }
    Some(message)
}

/// 四捨五入到小數 2 位（倍數顯示用）。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests;
