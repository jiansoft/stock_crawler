//! # 持股填息通知
//!
//! 每個交易日收盤資料確認完整後，檢查持股（含 ETF）近 [`LOOKBACK_DAYS`] 天內的每次除權息，
//! 收盤價**今天**第一次回到除權息前一日收盤（填息）的就發一則 Telegram。填息規則與
//! `/stocks/{symbol}/dividend-fill` 相同（[`fill::fill_outcome`]）。
//!
//! - 以「填息日就是今天」判斷，同一次除權息只會在填息當天出現一次，不需另外記錄已通知。
//! - 今天休市（最新收盤不是今天）時自然沒有任何填息日是今天，不發送。
//! - 單檔查詢失敗只記錄警告、略過該檔。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate};
use rust_decimal::Decimal;

use super::{format_decimal_with_commas, holdings};
use crate::{
    core::{alert, util::text},
    domain::dividend::fill::{self, DividendFillQuery, FillOutcome},
    infra::database::repository::dividend_fill::PgDividendFillRepository,
};

/// 只看除權息日在今天往前這麼多天內的事件；超過一年還沒填息的不再追蹤。
const LOOKBACK_DAYS: u64 = 365;

/// 往除權息日前多抓幾天日線，確保長假或停牌後仍找得到除權息前一日的收盤。
const PRE_EX_LOOKBACK_DAYS: u64 = 30;

/// 排程入口：通知今天填息的持股。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::active_holding_symbols()
        .await
        .context("fetch active holdings for dividend fill notification failed")?;
    let today = Local::now().date_naive();
    let filled = filled_today(&PgDividendFillRepository::new(), &symbols, today).await;
    tracing::info!(
        "持股填息通知: holdings={}, filled={}",
        symbols.len(),
        filled.len()
    );
    if let Some(message) = build_message(today, &filled, holdings::stock_name) {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 持股中填息日是 `today` 的除權息，依代號、除權息日排序；單檔失敗只記錄警告。
async fn filled_today(
    repository: &impl DividendFillQuery,
    symbols: impl IntoIterator<Item = &String>,
    today: NaiveDate,
) -> Vec<(String, FillOutcome)> {
    let mut filled = Vec::new();
    for symbol in symbols {
        match symbol_filled_today(repository, symbol, today).await {
            Ok(outcomes) => filled.extend(outcomes.into_iter().map(|o| (symbol.clone(), o))),
            Err(why) => tracing::warn!("持股填息檢查失敗: stock_symbol={symbol}, error={why:#}"),
        }
    }
    filled
}

/// 單一股票近 [`LOOKBACK_DAYS`] 天內、今天填息的除權息（由舊到新）。
async fn symbol_filled_today(
    repository: &impl DividendFillQuery,
    symbol: &str,
    today: NaiveDate,
) -> Result<Vec<FillOutcome>> {
    let since = today - Days::new(LOOKBACK_DAYS);
    let mut events: Vec<_> = repository
        .fetch_ex_right_events(symbol)
        .await?
        .into_iter()
        .filter(|event| event.ex_date > since && event.ex_date <= today)
        .collect();
    events.sort_by_key(|event| event.ex_date);
    let Some(oldest) = events.first() else {
        return Ok(Vec::new());
    };
    let closes = repository
        .fetch_daily_closes(symbol, oldest.ex_date - Days::new(PRE_EX_LOOKBACK_DAYS))
        .await?;
    Ok(events
        .into_iter()
        .filter_map(|event| fill::fill_outcome(event, &closes))
        .filter(|outcome| outcome.fill_date == Some(today))
        .collect())
}

/// 股利文字：現金與股票股利（元），沒有的那一種不列。
fn dividend_text(cash: Decimal, stock: Decimal) -> String {
    let mut parts = Vec::new();
    if cash > Decimal::ZERO {
        parts.push(format!("現金 {} 元", cash.normalize()));
    }
    if stock > Decimal::ZERO {
        parts.push(format!("股票 {} 元", stock.normalize()));
    }
    parts.join("、")
}

/// 組出通知訊息（Telegram MarkdownV2）；沒有填息的持股時回 `None`。
fn build_message(
    date: NaiveDate,
    filled: &[(String, FillOutcome)],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if filled.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(128 + filled.len() * 160);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持股填息︰{date}"))
    );
    for (symbol, outcome) in filled {
        let days = outcome.trading_days_to_fill.unwrap_or_default();
        let line = format!(
            "{symbol} {} 除權息 {}（{}）：前一日收盤 {}，今日收盤 {}，第 {days} 個交易日填息",
            stock_name(symbol),
            outcome.event.ex_date,
            dividend_text(outcome.event.cash_dividend, outcome.event.stock_dividend),
            format_decimal_with_commas(outcome.pre_ex_close),
            format_decimal_with_commas(outcome.latest_close),
        );
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    Some(message)
}

#[cfg(test)]
mod tests;
