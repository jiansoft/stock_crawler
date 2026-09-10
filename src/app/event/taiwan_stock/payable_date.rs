use std::{collections::BTreeMap, fmt::Write};

use anyhow::Result;
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::{
    core::{alert, util::text},
    domain::dividend::entity::StockDividendPayableDateInfo,
    domain::dividend::repository::DividendRepository,
    domain::portfolio::entity::StockOwnershipDetail,
    domain::portfolio::repository::PortfolioRepository,
    infra::database::repository::dividend::PgDividendRepository,
    infra::database::repository::portfolio::PgPortfolioRepository,
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
    stocks_payable_date_info: &[StockDividendPayableDateInfo],
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
        text::escape_markdown_v2(today.to_string())
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
            text::escape_markdown_v2(&batch.name),
            text::escape_markdown_v2(member_label(batch.member_id)),
            text::escape_markdown_v2(format_share_quantity(batch.share_quantity)),
            text::escape_markdown_v2(format_decimal_with_commas(batch.cash)),
            text::escape_markdown_v2(format_decimal_with_commas(batch.stock_money)),
            text::escape_markdown_v2(format_decimal_with_commas(batch.total()))
        );
    }

    Some(msg)
}

/// 組出「本日發放股利」的通知訊息。
///
/// 同一筆股利的現金與股票發放日可能不同天（例如現金 9/02、股票 9/10），
/// 因此只列出發放日確實落在今天的項目，合計也只加總今天入帳的部分。
fn build_payable_summary_message(
    today: NaiveDate,
    stocks_payable_date_info: &[StockDividendPayableDateInfo],
) -> Option<String> {
    let today_str = today.to_string();
    let mut msg = String::with_capacity(2048);

    if writeln!(
        &mut msg,
        "{} 進行股利發放的股票如下︰",
        text::escape_markdown_v2(&today_str)
    )
    .is_err()
    {
        return None;
    }

    let mut has_any = false;

    for stock in stocks_payable_date_info {
        let cash_payable_today = stock.payable_date1 == today_str;
        let stock_payable_today = stock.payable_date2 == today_str;
        if !cash_payable_today && !stock_payable_today {
            continue;
        }

        has_any = true;
        let _ = write!(
            &mut msg,
            "    {0} {1} ",
            stock.stock_symbol,
            text::escape_markdown_v2(&stock.name),
        );

        let mut sum = Decimal::ZERO;

        if cash_payable_today {
            sum += stock.cash_dividend;
            let _ = write!(
                &mut msg,
                "現金︰{0}元 ",
                text::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
            );
        }

        if stock_payable_today {
            sum += stock.stock_dividend;
            let _ = write!(
                &mut msg,
                "股票︰{0}元 ",
                text::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
            );
        }

        let _ = writeln!(
            &mut msg,
            "合計︰{0}元 ",
            text::escape_markdown_v2(sum.normalize().to_string()),
        );
    }

    has_any.then_some(msg)
}

/// 提提醒本日發放股利的股票(只通知自已有的股票)
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();
    let dividend_repo = PgDividendRepository::new();
    let stocks_payable_date_info = dividend_repo.fetch_payable_date_info_on_date(today).await?;
    if stocks_payable_date_info.is_empty() {
        return Ok(());
    }

    let stock_symbols: Vec<String> = stocks_payable_date_info
        .iter()
        .map(|stock| stock.stock_symbol.to_string())
        .collect();

    //群內通知
    if let Some(msg) = build_payable_summary_message(today, &stocks_payable_date_info) {
        alert::send_message(&msg).await;
    }

    let portfolio_repo = PgPortfolioRepository::new();
    let holdings = portfolio_repo
        .fetch_active_holdings(Some(stock_symbols))
        .await?;
    if let Some(batch_msg) =
        build_batch_dividend_message(today, &stocks_payable_date_info, &holdings)
    {
        alert::send_message(&batch_msg).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    use super::*;

    fn make_holding(
        serial: i64,
        member_id: i64,
        security_code: &str,
        share_quantity: i64,
        date: (i32, u32, u32),
    ) -> StockOwnershipDetail {
        StockOwnershipDetail {
            serial,
            member_id,
            security_code: security_code.to_string(),
            share_quantity,
            created_time: Local
                .with_ymd_and_hms(date.0, date.1, date.2, 0, 0, 0)
                .unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn test_build_batch_dividend_message_groups_by_stock_and_member() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();
        let stocks = vec![StockDividendPayableDateInfo {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            cash_dividend: dec!(3),
            stock_dividend: dec!(0.2),
            sum: dec!(3.2),
            payable_date1: "2026-08-20".to_string(),
            payable_date2: "2026-08-20".to_string(),
            ex_dividend_date1: "2026-07-15".to_string(),
            ex_dividend_date2: "2026-07-15".to_string(),
        }];
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

    #[test]
    fn test_build_payable_summary_message_only_lists_items_payable_today() {
        // 2834、2838 的現金早已發放，今天只發股票股利，訊息不應再列出現金。
        let today = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
        let stocks = vec![
            StockDividendPayableDateInfo {
                stock_symbol: "2834".to_string(),
                name: "臺企銀".to_string(),
                cash_dividend: dec!(0.3),
                stock_dividend: dec!(0.7),
                sum: dec!(1),
                payable_date1: "2026-09-02".to_string(),
                payable_date2: "2026-09-10".to_string(),
                ex_dividend_date1: "2026-08-04".to_string(),
                ex_dividend_date2: "2026-08-04".to_string(),
            },
            StockDividendPayableDateInfo {
                stock_symbol: "2838".to_string(),
                name: "聯邦銀".to_string(),
                cash_dividend: dec!(0.46),
                stock_dividend: dec!(0.6),
                sum: dec!(1.06),
                payable_date1: "2026-08-14".to_string(),
                payable_date2: "2026-09-10".to_string(),
                ex_dividend_date1: "2026-07-15".to_string(),
                ex_dividend_date2: "2026-07-15".to_string(),
            },
            StockDividendPayableDateInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                cash_dividend: dec!(3),
                stock_dividend: dec!(0.2),
                sum: dec!(3.2),
                payable_date1: "2026-09-10".to_string(),
                payable_date2: "2026-09-10".to_string(),
                ex_dividend_date1: "2026-08-01".to_string(),
                ex_dividend_date2: "2026-08-01".to_string(),
            },
        ];

        let msg = build_payable_summary_message(today, &stocks).unwrap();

        assert!(msg.contains(r"2834 臺企銀 股票︰0\.7元 合計︰0\.7元"));
        assert!(msg.contains(r"2838 聯邦銀 股票︰0\.6元 合計︰0\.6元"));
        // 現金與股票同日發放時兩者都要列出，合計為兩者相加。
        assert!(msg.contains(r"2330 台積電 現金︰3元 股票︰0\.2元 合計︰3\.2元"));
        assert!(!msg.contains("現金︰0"));
    }

    #[test]
    fn test_build_payable_summary_message_returns_none_when_nothing_payable_today() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
        let stocks = vec![StockDividendPayableDateInfo {
            stock_symbol: "2834".to_string(),
            name: "臺企銀".to_string(),
            cash_dividend: dec!(0.3),
            stock_dividend: dec!(0.7),
            sum: dec!(1),
            payable_date1: "2026-09-02".to_string(),
            payable_date2: "2026-09-11".to_string(),
            ex_dividend_date1: "2026-08-04".to_string(),
            ex_dividend_date2: "2026-08-04".to_string(),
        }];

        assert!(build_payable_summary_message(today, &stocks).is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn test_calculate() {
        dotenvy::dotenv().ok();
        tracing::info!("開始 execute");
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        let _ = execute().await;

        tracing::info!("結束 execute");
    }
}
