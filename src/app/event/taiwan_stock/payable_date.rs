//! 08:00 股利發放日通知：群內「本日發放股利」清單，以及持股今天入帳的明細與本月至今合計。
//!
//! 持股入帳取 [`PortfolioRepository::fetch_payout_allocations`](crate::domain::portfolio::repository::PortfolioRepository::fetch_payout_allocations)：已除權息的用除權息當天寫入的
//! 已領股利明細，所以除權息後才賣出、今天才發放的也會列入，金額與
//! `/portfolio/upcoming-payouts` 一致。

use std::{collections::BTreeMap, fmt::Write};

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use rust_decimal::Decimal;

// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::{
    core::{alert, util::text},
    domain::dividend::entity::StockDividendPayableDateInfo,
    domain::dividend::repository::DividendRepository,
    domain::portfolio::payout::{self, DividendPayout},
    domain::portfolio::repository::PortfolioRepository,
    infra::database::repository::dividend::PgDividendRepository,
    infra::database::repository::portfolio::PgPortfolioRepository,
};

use super::{format_decimal_with_commas, format_share_quantity, member_label};

/// 組出持股今天入帳的明細與本月至今的現金合計；今天沒有持股入帳時回 `None`。
///
/// `today_payouts` 與 `month_payouts` 都依會員分列（不合併），本月至今依會員加總。
fn build_batch_dividend_message(
    today: NaiveDate,
    today_payouts: &[DividendPayout],
    month_payouts: &[DividendPayout],
) -> Option<String> {
    if today_payouts.is_empty() {
        return None;
    }

    let mut msg = String::with_capacity(512 + today_payouts.len() * 160);
    let _ = writeln!(
        &mut msg,
        "{} 持股股利入帳如下︰",
        text::escape_markdown_v2(today.to_string())
    );
    for item in today_payouts {
        let mut line = format!(
            "{} {} 持股:{}股 現金:{}元",
            item.name,
            member_label(item.member_id),
            format_share_quantity(item.shares),
            format_decimal_with_commas(item.cash_amount)
        );
        if item.stock_shares > 0 {
            let _ = write!(line, " 配股:{}股", format_share_quantity(item.stock_shares));
        }
        if item.estimated {
            line.push_str("（以目前持股推算）");
        }
        let _ = writeln!(
            &mut msg,
            "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1}",
            item.stock_symbol,
            text::escape_markdown_v2(line)
        );
    }

    let mut by_member = BTreeMap::<i64, Decimal>::new();
    for item in month_payouts {
        *by_member.entry(item.member_id).or_default() += item.cash_amount;
    }
    let members: Vec<String> = by_member
        .iter()
        .map(|(member_id, cash)| {
            format!(
                "{} {}元",
                member_label(*member_id),
                format_decimal_with_commas(*cash)
            )
        })
        .collect();
    let _ = writeln!(
        &mut msg,
        "{}",
        text::escape_markdown_v2(format!(
            "本月至今入帳現金：{}，合計 {}元（未扣補充保費與匯費）",
            members.join("、"),
            format_decimal_with_commas(payout::total_cash(month_payouts))
        ))
    );

    Some(msg)
}

/// 組出「本日發放股利」的通知訊息。
///
/// 同一筆股利的現金與股票發放日可能不同天（例如現金 9/02、股票 9/10），
/// 因此只列出發放日確實落在今天的項目，合計也只加總今天入帳的部分。
fn build_payable_summary_message(
    today: NaiveDate,
    stocks_payable_date_info: &[StockDividendPayableDateInfo],
) -> Option<String> {
    let today_str = today.to_string();
    let mut msg = String::with_capacity(2048);

    if writeln!(
        &mut msg,
        "{} 進行股利發放的股票如下︰",
        text::escape_markdown_v2(&today_str)
    )
    .is_err()
    {
        return None;
    }

    let mut has_any = false;

    for stock in stocks_payable_date_info {
        let cash_payable_today = stock.payable_date1 == today_str;
        let stock_payable_today = stock.payable_date2 == today_str;
        if !cash_payable_today && !stock_payable_today {
            continue;
        }

        has_any = true;
        let _ = write!(
            &mut msg,
            "    {0} {1} ",
            stock.stock_symbol,
            text::escape_markdown_v2(&stock.name),
        );

        let mut sum = Decimal::ZERO;

        if cash_payable_today {
            sum += stock.cash_dividend;
            let _ = write!(
                &mut msg,
                "現金︰{0}元 ",
                text::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
            );
        }

        if stock_payable_today {
            sum += stock.stock_dividend;
            let _ = write!(
                &mut msg,
                "股票︰{0}元 ",
                text::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
            );
        }

        let _ = writeln!(
            &mut msg,
            "合計︰{0}元 ",
            text::escape_markdown_v2(sum.normalize().to_string()),
        );
    }

    has_any.then_some(msg)
}

/// 提醒本日發放股利的股票，並通知持股今天入帳的明細。
pub async fn execute() -> Result<()> {
    notify(Local::now().date_naive()).await
}

/// 以 `today` 為今天發送通知；整合測試以固定日期呼叫。
async fn notify(today: NaiveDate) -> Result<()> {
    let dividend_repo = PgDividendRepository::new();
    let stocks_payable_date_info = dividend_repo.fetch_payable_date_info_on_date(today).await?;
    //群內通知（只列目前仍持有的股票）
    if let Some(msg) = build_payable_summary_message(today, &stocks_payable_date_info) {
        alert::send_message(&msg).await;
    }

    // 持股入帳不受上面清單限制：除權息後已全部賣出的，今天仍會入帳。

    let month_start = today.with_day(1).unwrap_or(today);
    let allocations = PgPortfolioRepository::new()
        .fetch_payout_allocations(0, month_start, today, today)
        .await?;
    let month_payouts = payout::split_by_payable_date(&allocations, month_start, today);
    let today_payouts: Vec<DividendPayout> = month_payouts
        .iter()
        .filter(|item| item.payable_date == today)
        .cloned()
        .collect();
    if let Some(batch_msg) = build_batch_dividend_message(today, &today_payouts, &month_payouts) {
        alert::send_message(&batch_msg).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
