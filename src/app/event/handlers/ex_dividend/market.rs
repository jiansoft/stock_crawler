//! 市場除權息清單訊息：股票在前、ETF 在後，各群組依殖利率由高到低；每檔附歷年填息（[`FillHints`]）。

use std::fmt::Write;

use super::super::EventDispatcher;
use super::annualized_suffix;
use crate::app::calculation::dividend_fill_hints::FillHints;
use crate::core::declare::Industry;
use crate::core::util::text;
use crate::domain::dividend::entity::StockDividendInfo;

impl EventDispatcher {
    /// 判斷一筆除權息資料是否屬於 ETF。
    pub(super) fn is_etf(stock: &StockDividendInfo) -> bool {
        stock.stock_industry_id == Industry::ExchangeTradedFund.serial()
    }

    /// 依殖利率由高到低比較兩筆除權息資料。
    pub(super) fn compare_dividend_yield_desc(
        a: &StockDividendInfo,
        b: &StockDividendInfo,
    ) -> std::cmp::Ordering {
        b.dividend_yield
            .partial_cmp(&a.dividend_yield)
            .unwrap_or(std::cmp::Ordering::Equal)
    }

    /// 將市場清單排序成「股票在前、ETF 在後」，各群組內再按殖利率降序。
    pub(super) fn sort_market_dividend_info(stocks_dividend_info: &mut [StockDividendInfo]) {
        stocks_dividend_info.sort_by(|a, b| match (Self::is_etf(a), Self::is_etf(b)) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => Self::compare_dividend_yield_desc(a, b),
        });
    }

    /// 將同一類別的除權息清單行文字寫入 Telegram 訊息。
    pub(super) fn write_market_dividend_rows<'a>(
        msg: &mut String,
        title: &str,
        stocks: impl Iterator<Item = &'a StockDividendInfo>,
        hints: &FillHints,
    ) {
        let mut has_rows = false;
        for stock in stocks {
            if !has_rows {
                let _ = writeln!(msg, "{}︰", text::escape_markdown_v2(title));
                has_rows = true;
            }

            let hint = hints.text(stock).map_or_else(String::new, |hint| {
                format!(" {}", text::escape_markdown_v2(hint))
            });
            let _ = writeln!(
                msg,
                "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} 現金︰{2}元\\({6}%\\) 股票 {3}元 合計︰{4}元\\({7}%\\) 昨收價:{5} 現金殖利率:{6}% 殖利率:{7}%{8}{9}",
                stock.stock_symbol,
                text::escape_markdown_v2(&stock.name),
                text::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
                text::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
                text::escape_markdown_v2(stock.sum.normalize().to_string()),
                text::escape_markdown_v2(stock.closing_price.normalize().to_string()),
                text::escape_markdown_v2(stock.cash_dividend_yield.normalize().to_string()),
                text::escape_markdown_v2(stock.dividend_yield.normalize().to_string()),
                text::escape_markdown_v2(annualized_suffix(
                    stock.payout_frequency(),
                    stock.annualized_yield()
                )),
                hint
            );
        }
    }

    /// 組出指定日期的市場除權息清單訊息。
    pub(super) fn build_market_dividend_message(
        date: chrono::NaiveDate,
        title: &str,
        stocks_dividend_info: &[StockDividendInfo],
        hints: &FillHints,
    ) -> String {
        let mut msg = String::with_capacity(2048);
        if writeln!(
            &mut msg,
            "{} {}",
            text::escape_markdown_v2(date.to_string()),
            text::escape_markdown_v2(title)
        )
        .is_ok()
        {
            Self::write_market_dividend_rows(
                &mut msg,
                "股票",
                stocks_dividend_info
                    .iter()
                    .filter(|stock| !Self::is_etf(stock)),
                hints,
            );
            Self::write_market_dividend_rows(
                &mut msg,
                "ETF",
                stocks_dividend_info
                    .iter()
                    .filter(|stock| Self::is_etf(stock)),
                hints,
            );
        }

        msg
    }
}
