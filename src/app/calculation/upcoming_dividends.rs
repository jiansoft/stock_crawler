//! 持股未來已公告的除權息：逐日查除權息、只留持股，依合計股數算預估入帳與配股。
//!
//! 週六每週市場摘要的「下週持股除權息」與 `/portfolio/upcoming-dividends` 共用。除權息日沿用
//! 08:00 除權息清單的依日期查詢（[`DividendRepository::fetch_stocks_with_dividends_on_date`](crate::domain::dividend::repository::DividendRepository::fetch_stocks_with_dividends_on_date)），
//! 只含已公告除權息日的事件；不推估、不扣二代健保與稅。

use std::collections::BTreeMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::dividend::{entity::StockDividendInfo, repository::DividendRepository};

/// 只需要「某天有哪些除權息」，測試不必實作整個 [`DividendRepository`]。
#[async_trait]
pub trait ExDividendDayQuery: Send + Sync {
    /// 指定日期除權或除息的股票（含每股股利與參考收盤價）。
    async fn fetch_on(&self, date: NaiveDate) -> Result<Vec<StockDividendInfo>>;
}

#[async_trait]
impl<T: DividendRepository> ExDividendDayQuery for T {
    async fn fetch_on(&self, date: NaiveDate) -> Result<Vec<StockDividendInfo>> {
        self.fetch_stocks_with_dividends_on_date(date).await
    }
}

/// 一檔持股在某天的除權息。
#[derive(Debug, Clone, PartialEq)]
pub struct UpcomingDividend {
    /// 除權息日（這一列只含這天除息或除權的部分）。
    pub ex_date: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 這天除息的每股現金股利；這天不除息為 0。
    pub cash_dividend: Decimal,
    /// 這天除權的每股股票股利（元）；這天不除權為 0。
    pub stock_dividend: Decimal,
    /// 最新收盤價（找同殖利率組填息基準用；沒有報價為 0）。
    pub closing_price: Decimal,
    /// 合計持股股數。
    pub shares: i64,
}

impl UpcomingDividend {
    /// 預估現金入帳：股數 × 每股現金股利（未扣二代健保與稅）。
    #[must_use]
    pub fn cash_amount(&self) -> Decimal {
        self.cash_dividend * Decimal::from(self.shares)
    }

    /// 預估配股股數：股票股利以面額 10 元換算，不足一股的部分捨去。
    #[must_use]
    pub fn stock_shares(&self) -> Decimal {
        (self.stock_dividend * Decimal::from(self.shares) / Decimal::TEN).floor()
    }
}

/// 各代號的合計股數（同一檔分次買進、不同會員的持股合併）。
#[must_use]
pub fn total_shares(holdings: impl IntoIterator<Item = (String, i64)>) -> BTreeMap<String, i64> {
    let mut shares = BTreeMap::new();
    for (symbol, quantity) in holdings {
        *shares.entry(symbol).or_insert(0) += quantity;
    }
    shares
}

/// `from` 到 `to`（含）每天的除權息，只留 `shares` 裡的持股，依除權息日、代號排序。
///
/// 沒有持股時不查詢；單日查詢失敗只記錄警告、少那一天。
pub async fn upcoming_dividends(
    query: &impl ExDividendDayQuery,
    shares: &BTreeMap<String, i64>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<UpcomingDividend> {
    let mut items = Vec::new();
    if shares.is_empty() {
        return items;
    }
    let dates = from.iter_days().take_while(|date| *date <= to);
    for date in dates {
        match query.fetch_on(date).await {
            Ok(stocks) => {
                for stock in stocks {
                    if let Some(item) = held_dividend(date, stock, shares) {
                        items.push(item);
                    }
                }
            }
            Err(why) => tracing::warn!("讀取 {date} 持股除權息失敗: {why:#}"),
        }
    }
    items.sort_by(|a, b| {
        a.ex_date
            .cmp(&b.ex_date)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    items
}

/// 持股在這天的除息與除權；不是持股、或這天既不除息也不除權時為 `None`。
fn held_dividend(
    ex_date: NaiveDate,
    stock: StockDividendInfo,
    shares: &BTreeMap<String, i64>,
) -> Option<UpcomingDividend> {
    let held = *shares.get(&stock.stock_symbol)?;
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
        return None;
    }
    Some(UpcomingDividend {
        ex_date,
        stock_symbol: stock.stock_symbol,
        name: stock.name,
        cash_dividend,
        stock_dividend,
        closing_price: stock.closing_price,
        shares: held,
    })
}

#[cfg(test)]
mod tests;
