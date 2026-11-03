//! 每月 1 日 08:10 上個月持股股利入帳彙總：上月每筆入帳（全部會員合併，現金與配股）、各會員與
//! 合計現金、與去年同月比較，以及上個月所屬年度的累計現金。
//!
//! 入帳取 [`PortfolioRepository::fetch_payout_allocations`]（已領股利明細，除權息後才賣出的也算）；
//! 上個月已經過去，不會有以目前持股推算的入帳。上個月沒有入帳就不發。

use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Local, Months, NaiveDate};
use rust_decimal::Decimal;

use super::{format_decimal_with_commas, format_share_quantity, member_label};
use crate::{
    core::{alert, util::text},
    domain::portfolio::{
        payout::{self, DividendPayout},
        repository::PortfolioRepository,
    },
    infra::database::repository::portfolio::PgPortfolioRepository,
};

/// 上個月的入帳與對照數字。
#[derive(Debug, Clone, PartialEq)]
struct MonthlySummary {
    /// 上個月第一天。
    from: NaiveDate,
    /// 上個月最後一天。
    to: NaiveDate,
    /// 上個月的入帳，依會員分列。
    payouts: Vec<DividendPayout>,
    /// 去年同月的現金入帳合計。
    last_year_cash: Decimal,
    /// 上個月所屬年度 1 月 1 日到上個月底的現金入帳合計。
    year_to_date_cash: Decimal,
}

/// 排程入口：彙整上個月的持股股利入帳並發送。
///
/// # Errors
///
/// 查詢入帳失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    notify(&PgPortfolioRepository::new(), Local::now().date_naive()).await
}

/// 以 `today` 為今天彙整並發送；上個月沒有入帳時不發。
async fn notify(repo: &impl PortfolioRepository, today: NaiveDate) -> Result<()> {
    let summary = load_summary(repo, today).await?;
    let payout_count = summary.payouts.len();
    tracing::info!(
        "上月持股股利入帳彙總: from={}, payouts={}",
        summary.from,
        payout_count
    );
    if let Some(message) = build_message(&summary) {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// `today` 的上個月第一天與最後一天。
fn previous_month(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let this_month = today.with_day(1).unwrap_or(today);
    let from = this_month - Months::new(1);
    (from, this_month.pred_opt().unwrap_or(from))
}

/// 查詢上個月、去年同月與今年累計的入帳。
async fn load_summary(repo: &impl PortfolioRepository, today: NaiveDate) -> Result<MonthlySummary> {
    let (from, to) = previous_month(today);
    let payouts = payouts_between(repo, from, to, today).await?;
    let last_year_from = from - Months::new(12);
    let last_year_to = (last_year_from + Months::new(1))
        .pred_opt()
        .unwrap_or(last_year_from);
    let last_year = payouts_between(repo, last_year_from, last_year_to, today).await?;
    let year_start = NaiveDate::from_ymd_opt(from.year(), 1, 1).unwrap_or(from);
    let year_to_date = payouts_between(repo, year_start, to, today).await?;
    Ok(MonthlySummary {
        from,
        to,
        payouts,
        last_year_cash: payout::total_cash(&last_year),
        year_to_date_cash: payout::total_cash(&year_to_date),
    })
}

async fn payouts_between(
    repo: &impl PortfolioRepository,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<DividendPayout>> {
    let allocations = repo.fetch_payout_allocations(0, from, to, today).await?;
    Ok(payout::split_by_payable_date(&allocations, from, to))
}

/// 組出 MarkdownV2 訊息；上個月沒有入帳時回 `None`。
fn build_message(summary: &MonthlySummary) -> Option<String> {
    if summary.payouts.is_empty() {
        return None;
    }

    let total = payout::total_cash(&summary.payouts);
    let mut by_member = BTreeMap::<i64, Decimal>::new();
    for item in &summary.payouts {
        *by_member.entry(item.member_id).or_default() += item.cash_amount;
    }
    let members: Vec<String> = by_member
        .iter()
        .map(|(member_id, cash)| {
            format!(
                "{} {} 元",
                member_label(*member_id),
                format_decimal_with_commas(*cash)
            )
        })
        .collect();

    let mut lines = vec![format!("{} 持股股利入帳彙總", summary.from.format("%Y-%m"))];
    for item in payout::merge_members(summary.payouts.clone()) {
        lines.push(payout_line(&item));
    }
    lines.push(format!("各會員：{}", members.join("、")));
    lines.push(format!(
        "合計現金 {} 元；{}",
        format_decimal_with_commas(total),
        last_year_text(total, summary.last_year_cash)
    ));
    lines.push(format!(
        "{} 年累計現金 {} 元（截至 {}）",
        summary.from.year(),
        format_decimal_with_commas(summary.year_to_date_cash),
        summary.to.format("%m-%d")
    ));
    lines.push("金額未扣二代健保補充保費與匯費".to_owned());

    let mut message = String::with_capacity(lines.len() * 64);
    for line in lines {
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    Some(message)
}

/// 一筆入帳：發放日、代號、名稱、現金與配股。
fn payout_line(item: &DividendPayout) -> String {
    let mut parts = Vec::with_capacity(2);
    if item.cash_amount > Decimal::ZERO {
        parts.push(format!(
            "現金 {} 元",
            format_decimal_with_commas(item.cash_amount)
        ));
    }
    if item.stock_shares > 0 {
        parts.push(format!(
            "配股 {} 股",
            format_share_quantity(item.stock_shares)
        ));
    }
    format!(
        "• {} {} {} {}",
        item.payable_date.format("%m-%d"),
        item.stock_symbol,
        item.name,
        parts.join("、")
    )
}

/// 與去年同月比較的文字；去年同月沒有入帳時直接說明。
fn last_year_text(total: Decimal, last_year: Decimal) -> String {
    if last_year <= Decimal::ZERO {
        return "去年同月沒有入帳".to_owned();
    }
    let change = ((total - last_year) / last_year * Decimal::from(100))
        .round_dp(1)
        .normalize();
    let sign = if change > Decimal::ZERO { "+" } else { "" };
    format!(
        "去年同月 {} 元（{sign}{change}%）",
        format_decimal_with_commas(last_year)
    )
}

#[cfg(test)]
mod tests;
