use anyhow::Result;
use chrono::{Local, NaiveDate};

use crate::internal::{
    backfill,
    cache::{TtlCacheInner, TTL},
    calculation,
    database::table::{daily_quote, estimate::Estimate, last_daily_quotes, yield_rank::YieldRank},
    logging,
};

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
    backfill::quote::execute().await?;
    let daily_quote_count = daily_quote::fetch_count_by_date(date).await?;
    logging::info_file_async("抓取上市櫃收盤數據結束".to_string());

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
    Estimate::insert(date).await?;
    logging::info_file_async("計算便宜、合理、昂貴價的估算結束".to_string());

    // 重建指定日期的 yield_rank 表內的數據
    YieldRank::upsert(date).await?;
    logging::info_file_async("重建 yield_rank 表內的數據結束".to_string());

    // 計算帳戶內市值
    calculation::money_history::calculate_money_history(date).await?;
    logging::info_file_async("計算帳戶內市值結束".to_string());

    // 清除記憶與Redis內所有的快取
    TTL.clear();

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_run() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async("開始 event::taiwan_stock::closing::aggregate".to_string());

        let current_date = NaiveDate::parse_from_str("2023-08-07", "%Y-%m-%d").unwrap();

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
}
