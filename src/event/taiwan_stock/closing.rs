use std::fmt::Write;

use crate::{
    backfill,
    bot::{self, telegram::Telegram},
    cache::{TtlCacheInner, TTL},
    calculation, crawler,
    database::table::{
        daily_money_history_member::{
            DailyMoneyHistoryMember, DailyMoneyHistoryMemberWithPreviousTradingDay,
        },
        daily_quote, last_daily_quotes,
        yield_rank::YieldRank,
    },
    logging,
};
use anyhow::Result;
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use scopeguard::defer;

/// 台股收盤事件發生時要進行的事情
pub async fn execute() -> Result<()> {
    logging::info_file_async("台股收盤事件開始");
    defer! {
       logging::info_file_async("台股收盤事件結束");
    }

    let current_date: NaiveDate = Local::now().date_naive();
    let aggregate = aggregate(current_date);
    let index = backfill::taiwan_stock_index::execute();
    let (res_aggregation, res_index) = tokio::join!(aggregate, index);

    if let Err(why) = res_index {
        logging::error_file_async(format!(
            "Failed to taiwan_stock_index::execute() because {:#?}",
            why
        ));
    }

    if let Err(why) = res_aggregation {
        logging::error_file_async(format!("Failed to closing::aggregate() because {:#?}", why));
    }

    // 停止 trace 事件所使用的即時報價背景任務
    crate::event::trace::price_tasks::stop_price_tasks().await;

    crawler::flush_site_latency_stats();

    Ok(())
}

/// 股票收盤數據匯總
async fn aggregate(date: NaiveDate) -> Result<()> {
    //抓取上市櫃公司每日收盤資訊
    let daily_quote_count = backfill::quote::execute(date).await?;
    //logging::info_file_async("抓取上市櫃收盤數據結束".to_string());
    //let daily_quote_count = daily_quote::fetch_count_by_date(date).await?;
    logging::info_file_async(format!("抓取上市櫃收盤數據結束:{}", daily_quote_count));

    if daily_quote_count == 0 {
        return Ok(());
    }

    // 補上當日缺少的每日收盤數據
    let lack_daily_quotes_count = daily_quote::makeup_for_the_lack_daily_quotes(date).await?;
    logging::info_file_async(format!(
        "補上當日缺少的每日收盤數據結束:{:#?}",
        lack_daily_quotes_count
    ));

    // 計算均線
    calculation::daily_quotes::calculate_moving_average(date).await?;
    logging::info_file_async("計算均線結束".to_string());

    // 重建 last_daily_quotes 表內的數據
    last_daily_quotes::LastDailyQuotes::rebuild().await?;
    logging::info_file_async("重建 last_daily_quotes 表內的數據結束".to_string());

    // 計算便宜、合理、昂貴價的估算
    calculation::estimated_price::calculate_estimated_price(date).await?;
    logging::info_file_async("計算便宜、合理、昂貴價的估算結束".to_string());

    // 重建指定日期的 yield_rank 表內的數據
    YieldRank::upsert(date).await?;
    logging::info_file_async("重建 yield_rank 表內的數據結束".to_string());

    // 計算帳戶內市值
    calculation::money_history::calculate_money_history(date).await?;
    logging::info_file_async("計算帳戶內市值結束".to_string());

    // 清除記憶與Redis內所有的快取
    TTL.clear();

    //發送通知本日與前一個交易日的市值變化
    notify_money_change(date).await
}

fn member_label(member_id: i64) -> String {
    match member_id {
        1 => "Eddie".to_string(),
        2 => "Unice".to_string(),
        3 => "Hugo".to_string(),
        4 => "Aiden".to_string(),
        _ => format!("Member {}", member_id),
    }
}

fn add_thousand_separators(raw: &str) -> String {
    let (sign, digits) = if let Some(rest) = raw.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", raw)
    };
    let mut result = String::with_capacity(raw.len() + raw.len() / 3);

    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }

    let formatted: String = result.chars().rev().collect();
    format!("{sign}{formatted}")
}

fn format_decimal_with_commas(value: Decimal) -> String {
    let rounded = value.round_dp(2).to_string();
    let (integer_part, fractional_part) = rounded
        .split_once('.')
        .map_or((rounded.as_str(), ""), |(int_part, frac_part)| {
            (int_part, frac_part)
        });
    let formatted_integer = add_thousand_separators(integer_part);

    match fractional_part.len() {
        0 => format!("{formatted_integer}.00"),
        1 => format!("{formatted_integer}.{fractional_part}0"),
        _ => format!("{formatted_integer}.{}", &fractional_part[..2]),
    }
}

fn format_money_change_line(
    label: &str,
    market_value: Decimal,
    previous_market_value: Decimal,
) -> String {
    let diff = market_value - previous_market_value;
    let percentage = if previous_market_value.is_zero() {
        "N/A".to_string()
    } else {
        format_decimal_with_commas((diff / previous_market_value) * dec!(100))
    };

    format!(
        "{}:{} {} \\({}%\\)",
        Telegram::escape_markdown_v2(label),
        Telegram::escape_markdown_v2(format_decimal_with_commas(market_value)),
        Telegram::escape_markdown_v2(format_decimal_with_commas(diff)),
        Telegram::escape_markdown_v2(percentage),
    )
}

fn build_money_change_message(
    rows: &[DailyMoneyHistoryMemberWithPreviousTradingDay],
) -> Option<String> {
    let date = rows.first()?.date;
    let mut msg = String::with_capacity(256);
    let _ = writeln!(
        &mut msg,
        "{} 市值變化",
        Telegram::escape_markdown_v2(date.to_string())
    );

    if let Some(total_row) = rows.iter().find(|row| row.member_id == 0) {
        let _ = writeln!(
            &mut msg,
            "{}",
            format_money_change_line(
                "合計",
                total_row.market_value,
                total_row.previous_market_value
            )
        );
    }

    for row in rows.iter().filter(|row| row.member_id > 0) {
        let _ = writeln!(
            &mut msg,
            "{}",
            format_money_change_line(
                &member_label(row.member_id),
                row.market_value,
                row.previous_market_value,
            )
        );
    }

    Some(msg.trim_end().to_string())
}

async fn notify_money_change(date: NaiveDate) -> Result<()> {
    let rows = DailyMoneyHistoryMember::fetch_with_previous_trading_day(date).await?;
    if let Some(msg) = build_money_change_message(&rows) {
        bot::telegram::send(&msg).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};
    use std::time::Duration;

    use rust_decimal_macros::dec;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_aggregate() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async("開始 event::taiwan_stock::closing::aggregate".to_string());

        let current_date = NaiveDate::parse_from_str("2026-04-02", "%Y-%m-%d").unwrap();

        match aggregate(current_date).await {
            Ok(_) => {
                logging::debug_file_async(
                    "event::taiwan_stock::closing::aggregate 完成".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to event::taiwan_stock::closing::aggregate because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 event::taiwan_stock::closing::aggregate".to_string());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_notify_money_change() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async(
            "開始 event::taiwan_stock::closing::notify_money_change".to_string(),
        );

        let current_date = Local::now().date_naive();

        match notify_money_change(current_date).await {
            Ok(_) => {
                logging::debug_file_async(
                    "event::taiwan_stock::closing::notify_money_change 完成".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to event::taiwan_stock::closing::notify_money_change because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async(
            "結束 event::taiwan_stock::closing::notify_money_change".to_string(),
        );
    }

    #[test]
    fn test_build_money_change_message_includes_hugo() {
        let date = NaiveDate::parse_from_str("2026-04-02", "%Y-%m-%d").unwrap();
        let previous_date = NaiveDate::parse_from_str("2026-04-01", "%Y-%m-%d").unwrap();
        let rows = vec![
            DailyMoneyHistoryMemberWithPreviousTradingDay {
                date,
                previous_date: Some(previous_date),
                member_id: 0,
                market_value: dec!(4273187.20),
                previous_market_value: dec!(4053774.55),
            },
            DailyMoneyHistoryMemberWithPreviousTradingDay {
                date,
                previous_date: Some(previous_date),
                member_id: 1,
                market_value: dec!(2195395.10),
                previous_market_value: dec!(2207807.70),
            },
            DailyMoneyHistoryMemberWithPreviousTradingDay {
                date,
                previous_date: Some(previous_date),
                member_id: 2,
                market_value: dec!(1500000.00),
                previous_market_value: dec!(1400000.00),
            },
            DailyMoneyHistoryMemberWithPreviousTradingDay {
                date,
                previous_date: Some(previous_date),
                member_id: 3,
                market_value: dec!(577792.10),
                previous_market_value: dec!(445966.85),
            },
        ];

        let msg = build_money_change_message(&rows).expect("message should be built");

        assert!(msg.contains("合計"));
        assert!(msg.contains("Eddie"));
        assert!(msg.contains("Unice"));
        assert!(msg.contains("Hugo"));
        assert!(msg.contains("4,273,187\\.20"));
        assert!(msg.contains("577,792\\.10"));
        assert!(msg.contains("\\-12,412\\.60"));
    }

    #[test]
    fn test_format_decimal_with_commas() {
        assert_eq!(format_decimal_with_commas(dec!(4273187.20)), "4,273,187.20");
        assert_eq!(format_decimal_with_commas(dec!(-12412.6)), "-12,412.60");
        assert_eq!(format_decimal_with_commas(dec!(5.41)), "5.41");
        assert_eq!(format_decimal_with_commas(dec!(0)), "0.00");
    }
}
