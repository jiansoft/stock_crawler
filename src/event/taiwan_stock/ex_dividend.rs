use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::{
    bot,
    bot::telegram::Telegram,
    calculation,
    database::table::{dividend, stock_ownership_details::StockOwnershipDetail},
};

use super::{format_decimal_with_commas, format_share_quantity, member_label};

/// 依今日除權息事件與目前持股，組出第二則持股預估股利通知。
///
/// 計算規則：
/// 1. 只統計今天以前買入、且目前尚未賣出的持股。
/// 2. 同一檔股票若多人持有，按 `member_id` 分開統計。
/// 3. 只有今天真的發生除息/除權的欄位才納入本次預估。
fn build_holding_dividend_message(
    today: NaiveDate,
    stocks_dividend_info: &[dividend::extension::stock_dividend_info::StockDividendInfo],
    holdings: &[StockOwnershipDetail],
) -> Option<String> {
    // 先把今日除權息股票轉成查表結構，後續可用持股代號快速對應股利資料。
    let stock_info_map = stocks_dividend_info
        .iter()
        .map(|stock| (stock.stock_symbol.as_str(), stock))
        .collect::<std::collections::HashMap<_, _>>();
    // key = (股票代號, member_id)，value = (股票名稱, 持股股數合計, 持股成本合計, 現金股利合計, 股票股利合計)
    let mut grouped = BTreeMap::<(String, i64), (String, i64, Decimal, Decimal, Decimal)>::new();

    for holding in holdings
        .iter()
        .filter(|holding| holding.created_time.date_naive() < today)
    {
        // 持股代號不在今日除權息清單內時，直接略過。
        let Some(stock) = stock_info_map.get(holding.security_code.as_str()) else {
            continue;
        };

        let share_quantity = Decimal::from(holding.share_quantity);
        // 只有今天是除息日才計入現金股利；若今天只有除權，現金股利應為 0。
        let estimated_cash_dividend = if stock.is_cash_ex_dividend_today {
            stock.cash_dividend * share_quantity
        } else {
            Decimal::ZERO
        };
        // 只有今天是除權日才計入股票股利；若今天只有除息，股票股利應為 0。
        let estimated_stock_dividend = if stock.is_stock_ex_dividend_today {
            stock.stock_dividend * share_quantity
        } else {
            Decimal::ZERO
        };
        // 殖利率以目前每股成本為基準，而不是歷史買入總成本。
        let holding_cost = (holding.current_cost_per_share * share_quantity).abs();

        // 同一人同一檔若分多筆買進，這裡合併成一列訊息。
        // 每股成本以加權平均成本重算，避免不同批次成本不一致時訊息失真。
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

    // 第二則訊息專門呈現「自己目前持有的股票」在今日除權息可領多少。
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
        // 最後輸出時才 round，內部計算過程維持原始精度。
        let _ = writeln!(
            &mut msg,
            "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} {2} 持股:{3}股 成本:{4}元\\({5}元\\) 現金股利:{6}元 股票股利:{7}元 現金殖利率:{8}% 殖利率:{9}%",
            stock_symbol,
            Telegram::escape_markdown_v2(name),
            Telegram::escape_markdown_v2(member_label(member_id)),
            Telegram::escape_markdown_v2(format_share_quantity(share_quantity)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(holding_cost)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(current_cost_per_share)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(cash_dividend)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(stock_dividend)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(cash_yield)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(total_yield))
        );
    }

    Some(msg)
}

/// 發送今日除權息提醒，並在之後追加持股預估股利通知。
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();
    let mut stocks_dividend_info =
        dividend::extension::stock_dividend_info::fetch_stocks_with_dividends_on_date(today)
            .await?;

    if stocks_dividend_info.is_empty() {
        return Ok(());
    }
    // 按 殖利率 降序排序
    stocks_dividend_info.sort_by(|a, b| {
        b.dividend_yield
            .partial_cmp(&a.dividend_yield)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut stock_symbols: Vec<String> = Vec::with_capacity(stocks_dividend_info.len());
    let mut msg = String::with_capacity(2048);
    if writeln!(
        &mut msg,
        "{} 進行除權息的股票如下︰",
        Telegram::escape_markdown_v2(today.to_string())
    )
    .is_ok()
    {
        for stock in &stocks_dividend_info {
            // 第一則訊息是市場清單，列出今天有除權或除息的股票與殖利率。
            stock_symbols.push(stock.stock_symbol.to_string());
            let _ = writeln!(
                &mut msg,
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

    // 先發送市場清單，維持既有提醒順序。
    bot::telegram::send(&msg).await;
    // 再更新這批股票對應持股的股利記錄。
    calculation::dividend_record::execute(today.year(), Some(stock_symbols.clone())).await;

    // 重新讀取持股後，組第二則「分人分股」的預估股利通知。
    let holdings = StockOwnershipDetail::fetch(Some(stock_symbols)).await?;
    if let Some(holding_msg) =
        build_holding_dividend_message(today, &stocks_dividend_info, &holdings)
    {
        bot::telegram::send(&holding_msg).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::TimeZone;
    use rust_decimal_macros::dec;
    use tokio::time;

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        let _ = execute().await;

        logging::info_file_async("結束 execute".to_string());
        time::sleep(Duration::from_secs(1)).await;
    }

    #[test]
    fn test_build_holding_dividend_message_groups_by_stock_and_member() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let stocks = vec![
            dividend::extension::stock_dividend_info::StockDividendInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                cash_dividend: dec!(3.5),
                stock_dividend: dec!(0.2),
                sum: dec!(3.7),
                closing_price: dec!(950),
                dividend_yield: dec!(0.39),
                cash_dividend_yield: dec!(0.37),
                is_cash_ex_dividend_today: true,
                is_stock_ex_dividend_today: false,
            },
            dividend::extension::stock_dividend_info::StockDividendInfo {
                stock_symbol: "2317".to_string(),
                name: "鴻海".to_string(),
                cash_dividend: dec!(5),
                stock_dividend: dec!(0.3),
                sum: dec!(5.3),
                closing_price: dec!(150),
                dividend_yield: dec!(3.53),
                cash_dividend_yield: dec!(3.33),
                is_cash_ex_dividend_today: true,
                is_stock_ex_dividend_today: true,
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

        let msg = build_holding_dividend_message(today, &stocks, &holdings).unwrap();

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
    fn test_format_decimal_with_commas() {
        assert_eq!(format_decimal_with_commas(dec!(16638)), "16,638");
        assert_eq!(format_decimal_with_commas(dec!(83.19)), "83.19");
        assert_eq!(format_decimal_with_commas(dec!(1234567.8)), "1,234,567.8");
    }
}
