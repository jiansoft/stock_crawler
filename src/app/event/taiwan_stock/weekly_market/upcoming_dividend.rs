//! 每週市場摘要的「下週持股除權息」一段：下週一到週五持股（含 ETF）的除權息日、每股股利、
//! 預估入帳與近 5 年填息。
//!
//! 除權息日沿用 08:00 除權息清單的依日期查詢（[`DividendRepository::fetch_stocks_with_dividends_on_date`]），
//! 填息文字沿用 [`FillHints::text`]；只含已公告除權息日的事件，下週沒有就不列。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Datelike, Days, NaiveDate};
use rust_decimal::Decimal;

use super::super::format_decimal_with_commas;
use crate::{
    app::calculation::dividend_fill_hints::FillHints,
    domain::dividend::{entity::StockDividendInfo, repository::DividendRepository},
};

/// 週摘要只需要「某天有哪些除權息」，測試不必實作整個 [`DividendRepository`]。
#[async_trait]
pub(super) trait ExDividendDayQuery: Send + Sync {
    /// 指定日期除權或除息的股票（含每股股利與參考收盤價）。
    async fn fetch_on(&self, date: NaiveDate) -> Result<Vec<StockDividendInfo>>;
}

#[async_trait]
impl<T: DividendRepository> ExDividendDayQuery for T {
    async fn fetch_on(&self, date: NaiveDate) -> Result<Vec<StockDividendInfo>> {
        self.fetch_stocks_with_dividends_on_date(date).await
    }
}

/// 下週持股除權息一段。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UpcomingSection {
    /// 下週一。
    pub(super) from: NaiveDate,
    /// 下週五。
    pub(super) to: NaiveDate,
    /// 依除權息日、代號排序。
    pub(super) items: Vec<UpcomingDividend>,
}

/// 一檔持股在某天的除權息。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UpcomingDividend {
    pub(super) ex_date: NaiveDate,
    pub(super) stock_symbol: String,
    pub(super) name: String,
    /// 這天除息的每股現金股利；這天不除息為 0。
    pub(super) cash_dividend: Decimal,
    /// 這天除權的每股股票股利（元）；這天不除權為 0。
    pub(super) stock_dividend: Decimal,
    /// 全部會員合計持股股數。
    pub(super) shares: i64,
    /// 近 5 年填息或同殖利率組基準（[`FillHints::text`]）。
    pub(super) fill: Option<String>,
}

impl UpcomingDividend {
    /// 預估現金入帳：股數 × 每股現金股利（未扣二代健保與稅）。
    fn cash_amount(&self) -> Decimal {
        self.cash_dividend * Decimal::from(self.shares)
    }

    /// 預估配股股數：股票股利以面額 10 元換算，不足一股的部分捨去。
    fn stock_shares(&self) -> Decimal {
        (self.stock_dividend * Decimal::from(self.shares) / Decimal::TEN).floor()
    }
}

/// 下週一到週五；週六執行時是後天到下下週一的前一天。
pub(super) fn next_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let days_to_monday = 7 - u64::from(today.weekday().num_days_from_monday());
    let monday = today + Days::new(days_to_monday);
    (monday, monday + Days::new(4))
}

/// 各代號的合計股數（同一檔分次買進、不同會員的持股合併）。
pub(super) fn total_shares(
    holdings: impl IntoIterator<Item = (String, i64)>,
) -> BTreeMap<String, i64> {
    let mut shares = BTreeMap::new();
    for (symbol, quantity) in holdings {
        *shares.entry(symbol).or_insert(0) += quantity;
    }
    shares
}

/// 查詢下週每天的除權息，只留持股；單日查詢失敗只記錄警告。
pub(super) async fn upcoming_events(
    query: &impl ExDividendDayQuery,
    symbols: &BTreeSet<String>,
    today: NaiveDate,
) -> Vec<(NaiveDate, StockDividendInfo)> {
    if symbols.is_empty() {
        return Vec::new();
    }
    let (from, _) = next_week(today);
    let mut events = Vec::new();
    for date in from.iter_days().take(5) {
        match query.fetch_on(date).await {
            Ok(stocks) => events.extend(
                stocks
                    .into_iter()
                    .filter(|stock| symbols.contains(&stock.stock_symbol))
                    .map(|stock| (date, stock)),
            ),
            Err(why) => tracing::warn!("每週市場摘要：讀取 {date} 除權息失敗: {why:#}"),
        }
    }
    events
}

/// 組出下週持股除權息一段；沒有持股除權息時回 `None`。
pub(super) fn upcoming_section(
    events: Vec<(NaiveDate, StockDividendInfo)>,
    shares: &BTreeMap<String, i64>,
    hints: &FillHints,
    today: NaiveDate,
) -> Option<UpcomingSection> {
    let mut items: Vec<UpcomingDividend> = Vec::new();
    for (ex_date, stock) in events {
        let Some(&held) = shares.get(&stock.stock_symbol) else {
            continue;
        };
        let cash_dividend = if stock.is_cash_ex_dividend_on_date {
            stock.cash_dividend
        } else {
            Decimal::ZERO
        };
        let stock_dividend = if stock.is_stock_ex_dividend_on_date {
            stock.stock_dividend
        } else {
            Decimal::ZERO
        };
        if cash_dividend <= Decimal::ZERO && stock_dividend <= Decimal::ZERO {
            continue;
        }
        items.push(UpcomingDividend {
            ex_date,
            fill: hints.text(&stock),
            stock_symbol: stock.stock_symbol,
            name: stock.name,
            cash_dividend,
            stock_dividend,
            shares: held,
        });
    }
    if items.is_empty() {
        return None;
    }
    items.sort_by(|a, b| {
        a.ex_date
            .cmp(&b.ex_date)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    let (from, to) = next_week(today);
    Some(UpcomingSection { from, to, items })
}

/// 下週持股除權息一段的各行（開頭為空行）。
pub(super) fn lines(section: &UpcomingSection) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        format!(
            "下週持股除權息（{}～{}）",
            section.from.format("%m-%d"),
            section.to.format("%m-%d")
        ),
    ];
    for item in &section.items {
        let fill = item
            .fill
            .as_ref()
            .map_or_else(String::new, |text| format!("；{text}"));
        lines.push(format!(
            "• {} {} {}（{} 股）{}{fill}",
            item.ex_date.format("%m-%d"),
            item.stock_symbol,
            item.name,
            format_decimal_with_commas(Decimal::from(item.shares)),
            dividend_text(item)
        ));
    }
    let total: Decimal = section
        .items
        .iter()
        .map(UpcomingDividend::cash_amount)
        .sum();
    if section.items.len() > 1 && total > Decimal::ZERO {
        lines.push(format!(
            "• 合計預估現金入帳 {} 元（未扣二代健保與稅）",
            format_decimal_with_commas(total)
        ));
    }
    lines
}

/// 除息與除權的每股股利與預估入帳。
fn dividend_text(item: &UpcomingDividend) -> String {
    let mut parts = Vec::new();
    if item.cash_dividend > Decimal::ZERO {
        parts.push(format!(
            "除息 {} 元、預估入帳 {} 元",
            format_decimal_with_commas(item.cash_dividend),
            format_decimal_with_commas(item.cash_amount())
        ));
    }
    if item.stock_dividend > Decimal::ZERO {
        parts.push(format!(
            "除權 {} 元、預估配股 {} 股",
            format_decimal_with_commas(item.stock_dividend),
            format_decimal_with_commas(item.stock_shares())
        ));
    }
    parts.join("；")
}
