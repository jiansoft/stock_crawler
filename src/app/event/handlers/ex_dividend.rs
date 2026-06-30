//! `ExDividendReminderTriggered` 事件處理：發送除權息提醒、重新計算持股股利並發送通知。

use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::Result;
use chrono::Datelike;
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::{
    format_decimal_with_commas as format_decimal_flexible_commas, format_share_quantity,
    member_label,
};
use crate::core::declare::Industry;
use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::dividend::repository::DividendRepository;
use crate::domain::portfolio::entity::StockOwnershipDetail;
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database::repository::dividend::PgDividendRepository;
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::bot::telegram::Telegram;

use super::EventDispatcher;

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
            crate::interfaces::bot::telegram::send(&holding_msg).await;
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

    /// 判斷一筆除權息資料是否屬於 ETF。
    fn is_etf(stock: &StockDividendInfo) -> bool {
        stock.stock_industry_id == Industry::ExchangeTradedFund.serial()
    }

    /// 依殖利率由高到低比較兩筆除權息資料。
    fn compare_dividend_yield_desc(
        a: &StockDividendInfo,
        b: &StockDividendInfo,
    ) -> std::cmp::Ordering {
        b.dividend_yield
            .partial_cmp(&a.dividend_yield)
            .unwrap_or(std::cmp::Ordering::Equal)
    }

    /// 將市場清單排序成「股票在前、ETF 在後」，各群組內再按殖利率降序。
    fn sort_market_dividend_info(stocks_dividend_info: &mut [StockDividendInfo]) {
        stocks_dividend_info.sort_by(|a, b| match (Self::is_etf(a), Self::is_etf(b)) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => Self::compare_dividend_yield_desc(a, b),
        });
    }

    /// 將同一類別的除權息清單行文字寫入 Telegram 訊息。
    fn write_market_dividend_rows<'a>(
        msg: &mut String,
        title: &str,
        stocks: impl Iterator<Item = &'a StockDividendInfo>,
    ) {
        let mut has_rows = false;
        for stock in stocks {
            if !has_rows {
                let _ = writeln!(msg, "{}︰", Telegram::escape_markdown_v2(title));
                has_rows = true;
            }

            let _ = writeln!(
                msg,
                "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} 現金︰{2}元\\({6}%\\) 股票 {3}元 合計︰{4}元\\({7}%\\) 昨收價:{5} 現金殖利率:{6}% 殖利率:{7}%",
                stock.stock_symbol,
                Telegram::escape_markdown_v2(&stock.name),
                Telegram::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
                Telegram::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
                Telegram::escape_markdown_v2(stock.sum.normalize().to_string()),
                Telegram::escape_markdown_v2(stock.closing_price.normalize().to_string()),
                Telegram::escape_markdown_v2(stock.cash_dividend_yield.normalize().to_string()),
                Telegram::escape_markdown_v2(stock.dividend_yield.normalize().to_string())
            );
        }
    }

    /// 組出指定日期的市場除權息清單訊息。
    fn build_market_dividend_message(
        date: chrono::NaiveDate,
        title: &str,
        stocks_dividend_info: &[StockDividendInfo],
    ) -> String {
        let mut msg = String::with_capacity(2048);
        if writeln!(
            &mut msg,
            "{} {}",
            Telegram::escape_markdown_v2(date.to_string()),
            Telegram::escape_markdown_v2(title)
        )
        .is_ok()
        {
            Self::write_market_dividend_rows(
                &mut msg,
                "股票",
                stocks_dividend_info
                    .iter()
                    .filter(|stock| !Self::is_etf(stock)),
            );
            Self::write_market_dividend_rows(
                &mut msg,
                "ETF",
                stocks_dividend_info
                    .iter()
                    .filter(|stock| Self::is_etf(stock)),
            );
        }

        msg
    }

    /// 發送指定日期的市場除權息提醒。
    async fn send_market_dividend_message(
        date: chrono::NaiveDate,
        title: &str,
        stocks_dividend_info: &[StockDividendInfo],
    ) {
        if stocks_dividend_info.is_empty() {
            return;
        }

        let msg = Self::build_market_dividend_message(date, title, stocks_dividend_info);
        crate::interfaces::bot::telegram::send(&msg).await;
    }

    /// 依今日除權息事件與目前持股，組出第二則持股預估股利通知。
    fn build_holding_dividend_message(
        today: chrono::NaiveDate,
        stocks_dividend_info: &[StockDividendInfo],
        holdings: &[StockOwnershipDetail],
    ) -> Option<String> {
        let stock_info_map = stocks_dividend_info
            .iter()
            .map(|stock| (stock.stock_symbol.as_str(), stock))
            .collect::<std::collections::HashMap<_, _>>();
        let mut grouped =
            BTreeMap::<(String, i64), (String, i64, Decimal, Decimal, Decimal)>::new();

        for holding in holdings
            .iter()
            .filter(|holding| holding.created_time.date_naive() < today)
        {
            let Some(stock) = stock_info_map.get(holding.security_code.as_str()) else {
                continue;
            };

            let share_quantity = Decimal::from(holding.share_quantity);
            let estimated_cash_dividend = if stock.is_cash_ex_dividend_on_date {
                stock.cash_dividend * share_quantity
            } else {
                Decimal::ZERO
            };
            let estimated_stock_dividend = if stock.is_stock_ex_dividend_on_date {
                stock.stock_dividend * share_quantity
            } else {
                Decimal::ZERO
            };
            let holding_cost = (holding.current_cost_per_share * share_quantity).abs();

            let entry = grouped
                .entry((holding.security_code.clone(), holding.member_id))
                .or_insert_with(|| {
                    (
                        stock.name.clone(),
                        0,
                        Decimal::ZERO,
                        Decimal::ZERO,
                        Decimal::ZERO,
                    )
                });
            let total_shares = entry.1 + holding.share_quantity;
            let total_cost = entry.2 + holding_cost;
            entry.1 = total_shares;
            entry.2 = total_cost;
            entry.3 += estimated_cash_dividend;
            entry.4 += estimated_stock_dividend;
        }

        if grouped.is_empty() {
            return None;
        }

        let mut msg = String::with_capacity(2048);
        if writeln!(
            &mut msg,
            "{} 持股除權息預估如下︰",
            Telegram::escape_markdown_v2(today.to_string())
        )
        .is_err()
        {
            return None;
        }

        for (
            (stock_symbol, member_id),
            (name, share_quantity, holding_cost, cash_dividend, stock_dividend),
        ) in grouped
        {
            let cash_yield = if holding_cost.is_zero() {
                Decimal::ZERO
            } else {
                (cash_dividend / holding_cost) * Decimal::new(100, 0)
            };
            let total_yield = if holding_cost.is_zero() {
                Decimal::ZERO
            } else {
                ((cash_dividend + stock_dividend) / holding_cost) * Decimal::new(100, 0)
            };
            let current_cost_per_share = if share_quantity == 0 {
                Decimal::ZERO
            } else {
                holding_cost / Decimal::from(share_quantity)
            };

            let _ = writeln!(
                &mut msg,
                "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} {2} 持股:{3}股 成本:{4}元\\({5}元\\) 現金股利:{6}元 股票股利:{7}元 現金殖利率:{8}% 殖利率:{9}%",
                stock_symbol,
                Telegram::escape_markdown_v2(name),
                Telegram::escape_markdown_v2(member_label(member_id)),
                Telegram::escape_markdown_v2(format_share_quantity(share_quantity)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(holding_cost)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(
                    current_cost_per_share
                )),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(cash_dividend)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(stock_dividend)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(cash_yield)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(total_yield))
            );
        }

        Some(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn test_build_holding_dividend_message_groups_by_stock_and_member() {
        use crate::domain::portfolio::entity::StockOwnershipDetail;
        use chrono::{Local, NaiveDate, TimeZone};
        use rust_decimal_macros::dec;

        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let stocks = vec![
            StockDividendInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                stock_industry_id: crate::core::declare::Industry::Semiconductor.serial(),
                cash_dividend: dec!(3.5),
                stock_dividend: dec!(0.2),
                sum: dec!(3.7),
                closing_price: dec!(950),
                dividend_yield: dec!(0.39),
                cash_dividend_yield: dec!(0.37),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "2317".to_string(),
                name: "鴻海".to_string(),
                stock_industry_id: crate::core::declare::Industry::ElectronicComponents.serial(),
                cash_dividend: dec!(5),
                stock_dividend: dec!(0.3),
                sum: dec!(5.3),
                closing_price: dec!(150),
                dividend_yield: dec!(3.53),
                cash_dividend_yield: dec!(3.33),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: true,
            },
        ];
        let holdings = vec![
            StockOwnershipDetail {
                serial: 1,
                security_code: "2330".to_string(),
                member_id: 1,
                share_quantity: 1000,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(600),
                holding_cost: dec!(-600000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 3, 31, 9, 0, 0).unwrap(),
            },
            StockOwnershipDetail {
                serial: 2,
                security_code: "2330".to_string(),
                member_id: 2,
                share_quantity: 500,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(500),
                holding_cost: dec!(-250000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 3, 30, 9, 0, 0).unwrap(),
            },
            StockOwnershipDetail {
                serial: 3,
                security_code: "2317".to_string(),
                member_id: 2,
                share_quantity: 2000,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(120),
                holding_cost: dec!(-240000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 3, 20, 9, 0, 0).unwrap(),
            },
            StockOwnershipDetail {
                serial: 4,
                security_code: "2317".to_string(),
                member_id: 2,
                share_quantity: 1000,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(120),
                holding_cost: dec!(-120000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 4, 1, 9, 0, 0).unwrap(),
            },
        ];

        let msg =
            EventDispatcher::build_holding_dividend_message(today, &stocks, &holdings).unwrap();

        assert!(msg.contains("2330"));
        assert!(msg.contains("Eddie"));
        assert!(msg.contains("持股:1,000股"));
        assert!(msg.contains("成本:600,000元\\(600元\\)"));
        assert!(msg.contains("現金股利:3,500元"));
        assert!(msg.contains("股票股利:0元"));
        assert!(msg.contains("現金殖利率:0\\.58%"));
        assert!(msg.contains("殖利率:0\\.58%"));
        assert!(msg.contains("Unice"));
        assert!(msg.contains("持股:2,000股"));
        assert!(msg.contains("成本:240,000元\\(120元\\)"));
        assert!(msg.contains("現金股利:10,000元"));
        assert!(msg.contains("股票股利:600元"));
        assert!(msg.contains("現金殖利率:4\\.17%"));
        assert!(msg.contains("殖利率:4\\.42%"));
        assert!(!msg.contains("持股:3000股"));
    }

    #[test]
    fn test_market_dividend_message_groups_stocks_before_etfs_and_sorts_each_group_by_yield() {
        use chrono::NaiveDate;
        use rust_decimal_macros::dec;

        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let mut stocks = vec![
            StockDividendInfo {
                stock_symbol: "0050".to_string(),
                name: "元大台灣50".to_string(),
                stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
                cash_dividend: dec!(2),
                stock_dividend: Decimal::ZERO,
                sum: dec!(2),
                closing_price: dec!(100),
                dividend_yield: dec!(2),
                cash_dividend_yield: dec!(2),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "2317".to_string(),
                name: "鴻海".to_string(),
                stock_industry_id: crate::core::declare::Industry::ElectronicComponents.serial(),
                cash_dividend: dec!(5),
                stock_dividend: Decimal::ZERO,
                sum: dec!(5),
                closing_price: dec!(100),
                dividend_yield: dec!(5),
                cash_dividend_yield: dec!(5),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "00878".to_string(),
                name: "國泰永續高股息".to_string(),
                stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
                cash_dividend: dec!(3),
                stock_dividend: Decimal::ZERO,
                sum: dec!(3),
                closing_price: dec!(100),
                dividend_yield: dec!(3),
                cash_dividend_yield: dec!(3),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                stock_industry_id: crate::core::declare::Industry::Semiconductor.serial(),
                cash_dividend: dec!(1),
                stock_dividend: Decimal::ZERO,
                sum: dec!(1),
                closing_price: dec!(100),
                dividend_yield: dec!(1),
                cash_dividend_yield: dec!(1),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
        ];

        EventDispatcher::sort_market_dividend_info(&mut stocks);
        let msg = EventDispatcher::build_market_dividend_message(
            today,
            "進行除權息的股票與 ETF 如下︰",
            &stocks,
        );

        let stock_section = msg.find("股票︰").unwrap();
        let etf_section = msg.find("ETF︰").unwrap();
        let hon_hai = msg.find("2317").unwrap();
        let tsmc = msg.find("2330").unwrap();
        let high_yield_etf = msg.find("00878").unwrap();
        let low_yield_etf = msg.find("0050").unwrap();

        assert!(msg.contains("2026\\-04\\-01 進行除權息的股票與 ETF 如下︰"));
        assert!(stock_section < hon_hai);
        assert!(hon_hai < tsmc);
        assert!(tsmc < etf_section);
        assert!(etf_section < high_yield_etf);
        assert!(high_yield_etf < low_yield_etf);
    }
}
