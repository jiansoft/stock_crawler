//! 持股除權息預告：08:00 除權息提醒之後，持股（含 ETF）在今天之後 [`PREVIEW_TRADING_DAYS`] 個交易日內
//! 除權息的，發一則預告（除權息日、每股股利、預估入帳與配股、近 5 年填息）。
//!
//! 同一檔同一個除權息日只預告一次（Redis 旗標），所以多半在第 [`PREVIEW_TRADING_DAYS`] 個交易日前收到；
//! 較晚才公告的會在公告後的第一個交易日早上補發。除權息當天的預估股利由事件處理器另外發送。

use std::collections::HashSet;

use anyhow::Result;
use chrono::NaiveDate;

use super::super::holdings;
use super::super::upcoming_dividend::{UpcomingLine, item_lines, with_fill_hints};
use super::find_next_trading_day;
use crate::{
    app::calculation::{
        dividend_fill_hints::FillHints,
        upcoming_dividends::{UpcomingDividend, total_shares, upcoming_dividends},
    },
    core::{alert, util::text},
    infra::{database::repository::dividend::PgDividendRepository, nosql::redis::CLIENT},
};

/// 預告涵蓋今天之後幾個交易日。
const PREVIEW_TRADING_DAYS: usize = 3;

/// 已預告旗標保留天數：比預告窗口長，跨連假也不會在除權息前重複預告。
const NOTIFIED_TTL_SECONDS: usize = 14 * 24 * 60 * 60;

/// 今天之後第 1 個到第 [`PREVIEW_TRADING_DAYS`] 個交易日。
pub(super) fn preview_window(
    today: NaiveDate,
    holidays: &HashSet<NaiveDate>,
) -> (NaiveDate, NaiveDate) {
    let from = find_next_trading_day(today, holidays);
    let mut to = from;
    for _ in 1..PREVIEW_TRADING_DAYS {
        to = find_next_trading_day(to, holidays);
    }
    (from, to)
}

/// 已預告旗標的 key：代號＋除權息日。
fn notified_key(dividend: &UpcomingDividend) -> String {
    format!(
        "ex_dividend_preview:{}:{}",
        dividend.stock_symbol, dividend.ex_date
    )
}

/// 組出預告訊息（MarkdownV2）；沒有要預告的持股時回 `None`。
pub(super) fn preview_message(
    from: NaiveDate,
    to: NaiveDate,
    items: &[UpcomingLine],
) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut lines = vec![format!(
        "持股除權息預告（{}～{}）︰",
        from.format("%m-%d"),
        to.format("%m-%d")
    )];
    lines.extend(item_lines(items));
    lines.push("僅供參考，除權息日與股利以公告為準".to_owned());
    let escaped: Vec<String> = lines.iter().map(text::escape_markdown_v2).collect();
    Some(escaped.join("\n"))
}

/// 查預告窗口內持股的除權息，濾掉已預告過的，發送並記下旗標。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤；個別日期查詢失敗只記警告（見 [`upcoming_dividends`]）。
pub(super) async fn send(today: NaiveDate, holidays: &HashSet<NaiveDate>) -> Result<()> {
    let (from, to) = preview_window(today, holidays);
    let shares = total_shares(holdings::active_holding_quantities().await?);
    let dividends = upcoming_dividends(&PgDividendRepository::new(), &shares, from, to).await;

    let mut fresh = Vec::with_capacity(dividends.len());
    for dividend in dividends {
        let key = notified_key(&dividend);
        // 先占旗標再發送：同一天重跑不會重複；Redis 失敗時寧可重複也不要漏發。
        match CLIENT.set_if_absent(&key, true, NOTIFIED_TTL_SECONDS).await {
            Ok(false) => {}
            Ok(true) => fresh.push(dividend),
            Err(why) => {
                tracing::warn!("寫入除權息預告紀錄 {key} 失敗，視為尚未預告: {why:?}");
                fresh.push(dividend);
            }
        }
    }
    if fresh.is_empty() {
        return Ok(());
    }

    let symbols: Vec<String> = fresh
        .iter()
        .map(|dividend| dividend.stock_symbol.clone())
        .collect();
    let hints = FillHints::load(&symbols, today).await;
    if let Some(message) = preview_message(from, to, &with_fill_hints(fresh, &hints)) {
        alert::send_message(&message).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
