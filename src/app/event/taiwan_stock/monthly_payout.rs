//! 每月 1 日 08:10 上個月持股股利入帳彙總：上月每筆入帳（全部會員合併，現金與配股）、各會員與
//! 合計現金、與去年同月比較，以及上個月所屬年度的累計現金。
//!
//! 入帳取 [`PortfolioRepository::fetch_payout_allocations`]（已領股利明細，除權息後才賣出的也算）；
//! 上個月已經過去，不會有以目前持股推算的入帳。上個月沒有入帳就不發。
//!
//! 上個月底以前有股利預估快照（`dividend_forecast_snapshot`）時，另附該快照對上個月的預估誤差：
//! 已配對的預估與實際合計、漏領筆數（N59）；比對規則同 `/portfolio/dividend-forecast/accuracy`。

use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Local, Months, NaiveDate};
use rust_decimal::Decimal;

use super::{format_decimal_with_commas, format_share_quantity, member_label};
use crate::{
    core::{alert, util::text},
    domain::portfolio::{
        forecast_accuracy::{self, MonthAccuracy},
        payout::{self, DividendPayout},
        repository::PortfolioRepository,
    },
    infra::database::repository::portfolio::{PgPortfolioRepository, forecast},
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
    /// 上個月底以前最近一次快照對上個月的預估誤差；沒有快照或快照沒有上個月的預估時為 `None`。
    forecast: Option<SnapshotAccuracy>,
}

/// 某次快照對上個月的預估誤差。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapshotAccuracy {
    /// 快照日。
    snapshot_date: NaiveDate,
    /// 上個月的已配對與漏領彙總。
    month: MonthAccuracy,
}

/// 排程入口：彙整上個月的持股股利入帳並發送。
///
/// # Errors
///
/// 查詢入帳失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let today = Local::now().date_naive();
    // 預估誤差只是附加資訊：讀不到時記錄後照常發入帳彙總。
    let accuracy = match load_snapshot_accuracy(today).await {
        Ok(accuracy) => accuracy,
        Err(why) => {
            tracing::warn!("讀取上月股利預估誤差失敗，入帳通知不附誤差: {why:#}");
            None
        }
    };
    notify(&PgPortfolioRepository::new(), accuracy, today).await
}

/// 上個月底以前最近一次快照（全部會員）對上個月的預估誤差。
///
/// # Errors
///
/// 查詢快照或入帳失敗時回傳錯誤。
async fn load_snapshot_accuracy(today: NaiveDate) -> Result<Option<SnapshotAccuracy>> {
    let (from, to) = previous_month(today);
    let Some(snapshot_date) = forecast::latest_snapshot_on_or_before(to).await? else {
        return Ok(None);
    };
    let inputs = forecast::load_accuracy_inputs(snapshot_date, 0, today).await?;
    let accuracy = forecast_accuracy::compare(&inputs.forecasts, &inputs.actuals, today);
    Ok(
        forecast_accuracy::month_accuracy(&accuracy, from).map(|month| SnapshotAccuracy {
            snapshot_date,
            month,
        }),
    )
}

/// 以 `today` 為今天彙整並發送；上個月沒有入帳時不發。
async fn notify(
    repo: &impl PortfolioRepository,
    forecast: Option<SnapshotAccuracy>,
    today: NaiveDate,
) -> Result<()> {
    let mut summary = load_summary(repo, today).await?;
    summary.forecast = forecast;
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
        forecast: None,
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
    if let Some(forecast) = &summary.forecast {
        lines.push(forecast_text(forecast));
    }
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

/// 快照預估誤差的文字：已配對的預估、實際與差額，再加漏領筆數。
fn forecast_text(forecast: &SnapshotAccuracy) -> String {
    let month = &forecast.month;
    let mut parts = Vec::with_capacity(2);
    if month.matched_count > 0 {
        let difference = month.matched_difference();
        let sign = if difference > Decimal::ZERO { "+" } else { "" };
        let percent = month
            .matched_difference_percent()
            .map(|percent| format!("，{sign}{}%", percent.normalize()))
            .unwrap_or_default();
        parts.push(format!(
            "比對 {} 筆，預估 {} 元、實際 {} 元（{sign}{} 元{percent}）",
            month.matched_count,
            format_decimal_with_commas(month.matched_forecast),
            format_decimal_with_commas(month.matched_actual),
            format_decimal_with_commas(difference),
        ));
    }
    if month.missed_count > 0 {
        parts.push(format!(
            "預估有入帳但沒領到 {} 筆（預估 {} 元）",
            month.missed_count,
            format_decimal_with_commas(month.missed_forecast)
        ));
    }
    format!(
        "{} 快照預估誤差：{}",
        forecast.snapshot_date.format("%m-%d"),
        parts.join("；")
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
