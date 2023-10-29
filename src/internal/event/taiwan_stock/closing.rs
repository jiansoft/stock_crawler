use anyhow::Result;
use chrono::{Local, NaiveDate};
use rust_decimal_macros::dec;

use crate::internal::{
    backfill, bot,
    cache::{TtlCacheInner, TTL},
    calculation,
    database::table::{
        daily_money_history::extension::with_previous_trading_day_money_history::DailyMoneyHistoryWithPreviousTradingDayMoneyHistory,
        daily_quote, last_daily_quotes, yield_rank::YieldRank,
    },
};
use crate::logging;

/// 台股收盤事件發生時要進行的事情
pub async fn execute() -> Result<()> {
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

    Ok(())
}

/// 股票收盤數據匯總
async fn aggregate(date: NaiveDate) -> Result<()> {
    //抓取上市櫃公司每日收盤資訊
    let daily_quote_count = backfill::quote::execute().await?;
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

async fn notify_money_change(date: NaiveDate) -> Result<()> {
    let mh = DailyMoneyHistoryWithPreviousTradingDayMoneyHistory::fetch(date).await?;

    // Percentage = ((a-b)/b)*100
    let hundred = dec!(100);
    let sum_diff = mh.sum - mh.previous_sum;
    let sum_percentage = (sum_diff / mh.previous_sum) * hundred;
    let eddie_diff = mh.eddie - mh.previous_eddie;
    let eddie_percentage = (eddie_diff / mh.previous_eddie) * hundred;
    let unice_diff = mh.unice - mh.previous_unice;
    let unice_percentage = (unice_diff / mh.previous_unice) * hundred;
    let msg = format!(
        "{} 市值變化\n合計:{} {} ({}%)\nEddie:{} {} ({}%)\nUnice:{} {} ({}%)",
        date,
        mh.sum.round_dp(2),
        sum_diff.round_dp(2),
        sum_percentage.round_dp(2),
        mh.eddie.round_dp(2),
        eddie_diff.round_dp(2),
        eddie_percentage.round_dp(2),
        mh.unice.round_dp(2),
        unice_diff.round_dp(2),
        unice_percentage.round_dp(2),
    );

    bot::telegram::send(&msg).await
}

#[cfg(test)]
mod tests {
    use crate::{
        internal::cache::SHARE,
        logging
    };

    use super::*;

    #[tokio::test]
    async fn test_aggregate() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async("開始 event::taiwan_stock::closing::aggregate".to_string());

        let current_date = NaiveDate::parse_from_str("2023-09-06", "%Y-%m-%d").unwrap();

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
}
