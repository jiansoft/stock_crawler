//! 持股預估股利訊息：依今日除權息事件與目前持股，依股票與成員分組。

use std::collections::BTreeMap;
use std::fmt::Write;

use rust_decimal::Decimal;

use super::super::EventDispatcher;
use super::annualized_suffix;
use crate::app::event::taiwan_stock::{
    format_decimal_with_commas as format_decimal_flexible_commas, format_share_quantity,
    member_label,
};
use crate::core::util::text;
use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::portfolio::entity::StockOwnershipDetail;

impl EventDispatcher {
    /// 依今日除權息事件與目前持股，組出第二則持股預估股利通知。
    pub(super) fn build_holding_dividend_message(
        today: chrono::NaiveDate,
        stocks_dividend_info: &[StockDividendInfo],
        holdings: &[StockOwnershipDetail],
    ) -> Option<String> {
        let stock_info_map = stocks_dividend_info
            .iter()
            .map(|stock| (stock.stock_symbol.as_str(), stock))
            .collect::<std::collections::HashMap<_, _>>();
        let mut grouped = BTreeMap::<
            (String, i64),
            (
                String,
                i64,
                Decimal,
                Decimal,
                Decimal,
                Option<(u32, &'static str)>,
            ),
        >::new();

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
                        stock.payout_frequency(),
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
            text::escape_markdown_v2(today.to_string())
        )
        .is_err()
        {
            return None;
        }

        for (
            (stock_symbol, member_id),
            (name, share_quantity, holding_cost, cash_dividend, stock_dividend, frequency),
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
                "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} {2} 持股:{3}股 成本:{4}元\\({5}元\\) 現金股利:{6}元 股票股利:{7}元 現金殖利率:{8}% 殖利率:{9}%{10}",
                stock_symbol,
                text::escape_markdown_v2(name),
                text::escape_markdown_v2(member_label(member_id)),
                text::escape_markdown_v2(format_share_quantity(share_quantity)),
                text::escape_markdown_v2(format_decimal_flexible_commas(holding_cost)),
                text::escape_markdown_v2(format_decimal_flexible_commas(current_cost_per_share)),
                text::escape_markdown_v2(format_decimal_flexible_commas(cash_dividend)),
                text::escape_markdown_v2(format_decimal_flexible_commas(stock_dividend)),
                text::escape_markdown_v2(format_decimal_flexible_commas(cash_yield)),
                text::escape_markdown_v2(format_decimal_flexible_commas(total_yield)),
                text::escape_markdown_v2(annualized_suffix(
                    frequency,
                    frequency.map(|(times, _)| (total_yield * Decimal::from(times)).round_dp(2))
                ))
            );
        }

        Some(msg)
    }
}
