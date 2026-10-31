//! `ExDividendReminderTriggered` 事件處理：發送除權息提醒、重新計算持股股利並發送通知。
//!
//! 子模組：[`market`]（市場除權息清單訊息）、[`fill_history`]（清單每檔附的歷年填息）、
//! [`holding`]（持股預估股利訊息）。

use anyhow::Result;
use chrono::{Datelike, Local};
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::format_decimal_with_commas as format_decimal_flexible_commas;
// 通知走 core::alert（port）——app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向；
// 訊息內容的 MarkdownV2 跳脫在子模組用 core::util::text。
use crate::core::alert;
use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::dividend::repository::DividendRepository;
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database::repository::dividend::PgDividendRepository;
use crate::infra::database::repository::portfolio::PgPortfolioRepository;

use self::fill_history::FillHints;
use super::EventDispatcher;

mod fill_history;
mod holding;
mod market;

/// 一年多次配息時附加在殖利率後面的「 年化殖利率:8.47%（月配）」；年配時為空字串。
fn annualized_suffix(
    frequency: Option<(u32, &'static str)>,
    annualized_yield: Option<Decimal>,
) -> String {
    match (frequency, annualized_yield) {
        (Some((_, label)), Some(value)) => format!(
            " 年化殖利率:{}%（{label}）",
            format_decimal_flexible_commas(value)
        ),
        _ => String::new(),
    }
}

impl EventDispatcher {
    /// 處理 `ExDividendReminderTriggered` 事件：發送除權息提醒、重新計算持股股利並發送通知。
    pub(super) async fn handle_ex_dividend_reminder_triggered(
        date: chrono::NaiveDate,
        next_trading_date: chrono::NaiveDate,
    ) -> Result<()> {
        let dividend_repo = PgDividendRepository::new();
        // 取得本日市場除權息資料
        let mut stocks_dividend_info = dividend_repo
            .fetch_stocks_with_dividends_on_date(date)
            .await?;
        Self::sort_market_dividend_info(&mut stocks_dividend_info);

        // 發送今日市場清單
        Self::send_market_dividend_message(
            date,
            "進行除權息的股票與 ETF 如下︰",
            &stocks_dividend_info,
        )
        .await;

        let stock_symbols: Vec<String> = stocks_dividend_info
            .iter()
            .map(|stock| stock.stock_symbol.to_string())
            .collect();

        if stock_symbols.is_empty() {
            // 本日無除權息時，只發送下一個交易日的預定公告，並提早返回
            let mut next_stocks = dividend_repo
                .fetch_stocks_with_dividends_on_date(next_trading_date)
                .await?;
            Self::sort_market_dividend_info(&mut next_stocks);
            Self::send_market_dividend_message(
                next_trading_date,
                "預計進行除權息的股票與 ETF 如下︰",
                &next_stocks,
            )
            .await;
            return Ok(());
        }

        // 更新這批股票對應持股的股利記錄
        crate::app::calculation::dividend_record::execute(date.year(), Some(stock_symbols.clone()))
            .await;

        // 重新讀取持股後，組「分人分股」的預估股利通知
        let portfolio_repo = PgPortfolioRepository::new();
        let holdings = portfolio_repo
            .fetch_active_holdings(Some(stock_symbols))
            .await?;

        if let Some(holding_msg) =
            Self::build_holding_dividend_message(date, &stocks_dividend_info, &holdings)
        {
            alert::send_message(&holding_msg).await;
        }

        // 最後發送下一交易日的預訂除權息公告
        let mut next_stocks = dividend_repo
            .fetch_stocks_with_dividends_on_date(next_trading_date)
            .await?;
        Self::sort_market_dividend_info(&mut next_stocks);
        Self::send_market_dividend_message(
            next_trading_date,
            "預計進行除權息的股票與 ETF 如下︰",
            &next_stocks,
        )
        .await;

        Ok(())
    }

    /// 發送指定日期的市場除權息提醒，每檔附歷年填息；今天與下一交易日的清單都以今天為準查近 5 年，
    /// 全市場基準因此共用同一份快取。
    async fn send_market_dividend_message(
        date: chrono::NaiveDate,
        title: &str,
        stocks_dividend_info: &[StockDividendInfo],
    ) {
        if stocks_dividend_info.is_empty() {
            return;
        }

        let symbols: Vec<String> = stocks_dividend_info
            .iter()
            .map(|stock| stock.stock_symbol.clone())
            .collect();
        let hints = FillHints::load(&symbols, Local::now().date_naive()).await;
        let msg = Self::build_market_dividend_message(date, title, stocks_dividend_info, &hints);
        alert::send_message(&msg).await;
    }
}

#[cfg(test)]
mod tests;
