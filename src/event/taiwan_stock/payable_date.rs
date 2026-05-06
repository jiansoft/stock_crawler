use std::{collections::BTreeMap, fmt::Write};

use anyhow::Result;
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use crate::{
    bot::{self, telegram::Telegram},
    database::table::{dividend, stock_ownership_details::StockOwnershipDetail},
};

use super::{format_decimal_with_commas, format_share_quantity, member_label};

fn is_holding_eligible_for_ex_date(holding_date: NaiveDate, ex_date: &str) -> bool {
    let Ok(ex_date) = NaiveDate::parse_from_str(ex_date, "%Y-%m-%d") else {
        return false;
    };

    holding_date < ex_date
}

#[derive(Debug, Clone)]
struct PayableBatchDividend {
    name: String,
    member_id: i64,
    share_quantity: i64,
    cash: Decimal,
    stock_money: Decimal,
}

impl PayableBatchDividend {
    fn total(&self) -> Decimal {
        self.cash + self.stock_money
    }
}

fn build_batch_dividend_message(
    today: NaiveDate,
    stocks_payable_date_info: &[dividend::extension::stock_dividend_payable_date_info::StockDividendPayableDateInfo],
    holdings: &[StockOwnershipDetail],
) -> Option<String> {
    let mut grouped = BTreeMap::<(String, i64), PayableBatchDividend>::new();

    for stock in stocks_payable_date_info {
        for holding in holdings
            .iter()
            .filter(|holding| holding.security_code == stock.stock_symbol)
        {
            let holding_date = holding.created_time.date_naive();
            let share_quantity = Decimal::from(holding.share_quantity);
            let cash = if stock.payable_date1 == today.to_string()
                && is_holding_eligible_for_ex_date(holding_date, &stock.ex_dividend_date1)
            {
                stock.cash_dividend * share_quantity
            } else {
                Decimal::ZERO
            };
            let stock_money = if stock.payable_date2 == today.to_string()
                && is_holding_eligible_for_ex_date(holding_date, &stock.ex_dividend_date2)
            {
                stock.stock_dividend * share_quantity
            } else {
                Decimal::ZERO
            };

            if cash.is_zero() && stock_money.is_zero() {
                continue;
            }

            let entry = grouped
                .entry((stock.stock_symbol.clone(), holding.member_id))
                .or_insert_with(|| PayableBatchDividend {
                    name: stock.name.clone(),
                    member_id: holding.member_id,
                    share_quantity: 0,
                    cash: Decimal::ZERO,
                    stock_money: Decimal::ZERO,
                });

            entry.share_quantity += holding.share_quantity;
            entry.cash += cash;
            entry.stock_money += stock_money;
        }
    }

    if grouped.is_empty() {
        return None;
    }

    let mut msg = String::with_capacity(2048);
    if writeln!(
        &mut msg,
        "{} 持股批次預估入帳如下︰",
        Telegram::escape_markdown_v2(today.to_string())
    )
    .is_err()
    {
        return None;
    }

    for ((stock_symbol, _), batch) in grouped {
        let _ = writeln!(
            &mut msg,
            "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} {2} 持股:{3}股 現金:{4}元 股票:{5}元 合計:{6}元",
            stock_symbol,
            Telegram::escape_markdown_v2(&batch.name),
            Telegram::escape_markdown_v2(member_label(batch.member_id)),
            Telegram::escape_markdown_v2(format_share_quantity(batch.share_quantity)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(batch.cash)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(batch.stock_money)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(batch.total()))
        );
    }

    Some(msg)
}

/// 提提醒本日發放股利的股票(只通知自已有的股票)
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();
    let stocks_payable_date_info =
        dividend::extension::stock_dividend_payable_date_info::fetch(today).await?;
    if stocks_payable_date_info.is_empty() {
        return Ok(());
    }

    let mut stock_symbols: Vec<String> = Vec::with_capacity(stocks_payable_date_info.len());
    let mut msg = String::with_capacity(2048);

    if writeln!(
        &mut msg,
        "{} 進行股利發放的股票如下︰",
        Telegram::escape_markdown_v2(today.to_string())
    )
    .is_ok()
    {
        for stock in &stocks_payable_date_info {
            stock_symbols.push(stock.stock_symbol.to_string());
            let _ = write!(
                &mut msg,
                "    {0} {1} ",
                stock.stock_symbol,
                Telegram::escape_markdown_v2(&stock.name),
            );

            if stock.payable_date1 != "-" {
                let _ = write!(
                    &mut msg,
                    "現金︰{0}元 ",
                    Telegram::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
                );
            }

            if stock.payable_date2 != "-" {
                let _ = write!(
                    &mut msg,
                    "股票︰{0}元 ",
                    Telegram::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
                );
            }

            let _ = writeln!(
                &mut msg,
                "合計︰{0}元 ",
                Telegram::escape_markdown_v2(stock.sum.normalize().to_string()),
            );
        }
    }

    //群內通知
    bot::telegram::send(&msg).await;

    let holdings = StockOwnershipDetail::fetch(Some(stock_symbols)).await?;
    if let Some(batch_msg) =
        build_batch_dividend_message(today, &stocks_payable_date_info, &holdings)
    {
        bot::telegram::send(&batch_msg).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    use crate::logging;

    use super::*;

    fn make_holding(
        serial: i64,
        member_id: i64,
        security_code: &str,
        share_quantity: i64,
        date: (i32, u32, u32),
    ) -> StockOwnershipDetail {
        let mut holding = StockOwnershipDetail::new();
        holding.serial = serial;
        holding.member_id = member_id;
        holding.security_code = security_code.to_string();
        holding.share_quantity = share_quantity;
        holding.created_time = Local
            .with_ymd_and_hms(date.0, date.1, date.2, 0, 0, 0)
            .unwrap();
        holding
    }

    #[test]
    fn test_build_batch_dividend_message_groups_by_stock_and_member() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();
        let stocks = vec![
            dividend::extension::stock_dividend_payable_date_info::StockDividendPayableDateInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                cash_dividend: dec!(3),
                stock_dividend: dec!(0.2),
                sum: dec!(3.2),
                payable_date1: "2026-08-20".to_string(),
                payable_date2: "2026-08-20".to_string(),
                ex_dividend_date1: "2026-07-15".to_string(),
                ex_dividend_date2: "2026-07-15".to_string(),
            },
        ];
        let holdings = vec![
            make_holding(11, 1, "2330", 1000, (2026, 7, 14)),
            make_holding(12, 2, "2330", 500, (2026, 7, 14)),
            make_holding(13, 1, "2330", 300, (2026, 7, 14)),
            make_holding(13, 1, "2330", 300, (2026, 7, 15)),
        ];

        let msg = build_batch_dividend_message(today, &stocks, &holdings).unwrap();

        assert!(msg.contains("Eddie"));
        assert!(msg.contains("Unice"));
        assert!(!msg.contains("批次:"));
        assert!(!msg.contains("買進日:"));
        assert!(msg.contains("持股:1,300股"));
        assert!(msg.contains("現金:3,900元"));
        assert!(msg.contains("股票:260元"));
        assert!(msg.contains("現金:1,500元"));
        assert!(msg.contains("股票:100元"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        let _ = execute().await;

        logging::info_file_async("結束 execute".to_string());
    }
}
