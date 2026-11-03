//! 每週市場摘要的「下週持股除權息」、「下週持股股利入帳」與「本月持股股利入帳」三段：下週一到週五
//! 持股（含 ETF）的除權息日、每股股利、預估入帳與近 5 年填息，這幾天發放到帳的持股股利，以及本月
//! 已入帳／待入帳的現金與去年同月對照。
//!
//! 除權息每檔一行的文字與 08:00 持股除權息預告共用 [`super::super::upcoming_dividend`]；入帳取
//! [`PortfolioRepository::fetch_payout_allocations`]（已領明細，沒有時以目前持股推算），與
//! `/portfolio/upcoming-payouts`、`/portfolio/dividend-income` 相同。各段沒有資料就不列。

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::Decimal;

pub(super) use super::super::upcoming_dividend::UpcomingLine;
use super::super::upcoming_dividend::{item_lines, with_fill_hints};
use super::super::{format_decimal_with_commas, format_share_quantity};
use crate::app::calculation::{
    dividend_fill_hints::FillHints,
    upcoming_dividends::{ExDividendDayQuery, UpcomingDividend, upcoming_dividends},
};
use crate::domain::portfolio::{
    payout::{self, DividendPayout},
    repository::PortfolioRepository,
};

/// 下週持股除權息與股利入帳。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UpcomingSection {
    /// 下週一。
    pub(super) from: NaiveDate,
    /// 下週五。
    pub(super) to: NaiveDate,
    /// 除權息，依除權息日、代號排序。
    pub(super) items: Vec<UpcomingLine>,
    /// 股利入帳（全部會員合併），依發放日、代號排序。
    pub(super) payouts: Vec<DividendPayout>,
    /// 本月股利入帳與去年同月；兩者都沒有現金入帳時為 `None`。
    pub(super) month: Option<MonthIncome>,
}

/// 本月持股現金股利入帳（全部會員合併，依發放日歸月）與去年同月。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MonthIncome {
    /// 月份（1～12）。
    pub(super) month: u32,
    /// 發放日不晚於今天（已入帳）的現金。
    pub(super) received: Decimal,
    /// 發放日在今天之後（待入帳）的現金。
    pub(super) pending: Decimal,
    /// 去年同月的現金入帳。
    pub(super) last_year: Decimal,
}

/// 下週一到週五；週六執行時是後天到下下週一的前一天。
pub(super) fn next_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let days_to_monday = 7 - u64::from(today.weekday().num_days_from_monday());
    let monday = today + Days::new(days_to_monday);
    (monday, monday + Days::new(4))
}

/// 載入下週一段：持股除權息（附近 5 年填息）與股利入帳；入帳查詢失敗只少入帳那段。
pub(super) async fn load_upcoming(
    dividends_query: &impl ExDividendDayQuery,
    portfolio: &impl PortfolioRepository,
    shares: &BTreeMap<String, i64>,
    today: NaiveDate,
) -> Option<UpcomingSection> {
    let (from, to) = next_week(today);
    let dividends = upcoming_dividends(dividends_query, shares, from, to).await;
    let hints = if dividends.is_empty() {
        FillHints::default()
    } else {
        let symbols: Vec<String> = dividends
            .iter()
            .map(|dividend| dividend.stock_symbol.clone())
            .collect();
        FillHints::load(&symbols, today).await
    };
    let payouts = next_week_payouts(portfolio, today)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("每週市場摘要：讀取下週持股股利入帳失敗: {why:#}");
            Vec::new()
        });
    let month = month_income(portfolio, today).await.unwrap_or_else(|why| {
        tracing::warn!("每週市場摘要：讀取本月持股股利入帳失敗: {why:#}");
        None
    });
    upcoming_section(dividends, &hints, today, payouts, month)
}

/// 查詢 `today` 所在月份與去年同月的持股股利入帳（全部會員合併）；兩個月都沒有現金入帳時回 `None`。
///
/// # Errors
///
/// 任一查詢失敗時回傳錯誤。
pub(super) async fn month_income(
    repo: &impl PortfolioRepository,
    today: NaiveDate,
) -> Result<Option<MonthIncome>> {
    let first = today - Days::new(u64::from(today.day0()));
    let this_month = payouts_between(repo, first, today).await?;
    let last_year = payouts_between(repo, first - Months::new(12), today).await?;
    let (received, pending): (Vec<_>, Vec<_>) = this_month
        .into_iter()
        .partition(|payout| payout.payable_date <= today);
    let income = MonthIncome {
        month: today.month(),
        received: payout::total_cash(&received),
        pending: payout::total_cash(&pending),
        last_year: payout::total_cash(&last_year),
    };
    let empty = income.received + income.pending + income.last_year == Decimal::ZERO;
    Ok((!empty).then_some(income))
}

/// `first`（某月 1 日）整個月的入帳，全部會員合併。
async fn payouts_between(
    repo: &impl PortfolioRepository,
    first: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<DividendPayout>> {
    let last = first + Months::new(1) - Days::new(1);
    let allocations = repo.fetch_payout_allocations(0, first, last, today).await?;
    Ok(payout::merge_members(payout::split_by_payable_date(
        &allocations,
        first,
        last,
    )))
}

/// 查詢下週發放的持股股利入帳（全部會員合併）。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub(super) async fn next_week_payouts(
    repo: &impl PortfolioRepository,
    today: NaiveDate,
) -> Result<Vec<DividendPayout>> {
    let (from, to) = next_week(today);
    let allocations = repo.fetch_payout_allocations(0, from, to, today).await?;
    Ok(payout::merge_members(payout::split_by_payable_date(
        &allocations,
        from,
        to,
    )))
}

/// 組出下週一段，除權息每檔附填息文字；除權息、下週入帳與本月入帳都沒有時回 `None`。
pub(super) fn upcoming_section(
    dividends: Vec<UpcomingDividend>,
    hints: &FillHints,
    today: NaiveDate,
    payouts: Vec<DividendPayout>,
    month: Option<MonthIncome>,
) -> Option<UpcomingSection> {
    if dividends.is_empty() && payouts.is_empty() && month.is_none() {
        return None;
    }
    let (from, to) = next_week(today);
    Some(UpcomingSection {
        from,
        to,
        items: with_fill_hints(dividends, hints),
        payouts,
        month,
    })
}

/// 下週一段的各行（每段開頭為空行）。
pub(super) fn lines(section: &UpcomingSection) -> Vec<String> {
    let period = format!(
        "{}～{}",
        section.from.format("%m-%d"),
        section.to.format("%m-%d")
    );
    let mut lines = Vec::new();
    if !section.items.is_empty() {
        lines.push(String::new());
        lines.push(format!("下週持股除權息（{period}）"));
        lines.extend(item_lines(&section.items));
    }
    if !section.payouts.is_empty() {
        lines.push(String::new());
        lines.push(format!("下週持股股利入帳（{period}）"));
        lines.extend(section.payouts.iter().map(payout_line));
        lines.push(format!(
            "• 合計現金入帳 {} 元（未扣補充保費與匯費）",
            format_decimal_with_commas(payout::total_cash(&section.payouts))
        ));
    }
    if let Some(month) = section.month {
        lines.push(String::new());
        lines.push(format!("本月持股股利入帳（{} 月）", month.month));
        lines.push(format!(
            "• 已入帳 {} 元、待入帳 {} 元；去年同月 {} 元",
            format_decimal_with_commas(month.received),
            format_decimal_with_commas(month.pending),
            format_decimal_with_commas(month.last_year)
        ));
    }
    lines
}

/// 一筆入帳：發放日、代號、名稱、股數、現金與配股，推算的附註記。
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
    let estimated = if item.estimated {
        "（以目前持股推算）"
    } else {
        ""
    };
    format!(
        "• {} {} {}（{} 股）{}{estimated}",
        item.payable_date.format("%m-%d"),
        item.stock_symbol,
        item.name,
        format_share_quantity(item.shares),
        parts.join("、")
    )
}
