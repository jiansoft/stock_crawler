//use core::result::Result::Ok;

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use futures::{stream, StreamExt};
use rust_decimal::Decimal;


use crate::internal::cache::SHARE;
use crate::internal::database::table::quote_history_record::QuoteHistoryRecord;
use crate::internal::{
    database::table::{daily_quote, daily_quote::DailyQuote},
    logging, util,
};

/// 計算每家公司指定日期的均線值
pub async fn calculate_moving_average(date: NaiveDate) -> Result<()> {
    /*var (
        dq          = &database.DailyQuote{Date: date.Format("2006-01-02")}
        dqs, err    = dq.FetchDayQuotesByDate()
        workerCount = runtime.NumCPU() * 2
        wg          = &sync.WaitGroup{}
    )*/
    let quotes = daily_quote::fetch_daily_quotes_by_date(date).await?;

    stream::iter(quotes)
        .for_each_concurrent(util::concurrent_limit_32(), |dq| async move {
            if let Err(why) = process_daily_quote(dq).await {
                logging::error_file_async(format!(
                    "Failed to moving_average::calculate because {:?}",
                    why
                ));
            }
        })
        .await;
    Ok(())
}

pub(crate) async fn process_daily_quote(mut dq: DailyQuote) -> Result<()> {
    dq.fetch_moving_average().await?;
    //計算本日的股價淨值比 = 每股股價 ÷ 每股淨值
    match SHARE.stocks.read() {
        Ok(stocks_cache) => match stocks_cache.get(&dq.security_code) {
            Some(stock) => {
                let net_asset_value_per_share = stock.net_asset_value_per_share;
                if net_asset_value_per_share > Decimal::ZERO && dq.closing_price > Decimal::ZERO {
                    dq.price_to_book_ratio = dq.closing_price / net_asset_value_per_share;
                } else {
                    dq.price_to_book_ratio = Decimal::ZERO;
                }
            }
            None => {
                dq.price_to_book_ratio = Decimal::ZERO;
            }
        },
        Err(why) => {
            return Err(anyhow!("Failed to read stocks cache because {:?}", why));
        }
    };

    if dq.security_code == "2330" {
        dbg!(&dq);
    }

    dq.update_moving_average().await?;

    match SHARE.quote_history_records.write() {
        Ok(mut quote_history_records_guard) => {
            match quote_history_records_guard.get_mut(&dq.security_code) {
                None => {}
                Some(qhr) => {
                    //目前最高價小於歷史最高價
                    if dq.highest_price < qhr.maximum_price &&
                        //目前最低價高於歷史最低價
                        dq.lowest_price > qhr.minimum_price &&
                        //目前淨值比小於歷史最高淨值比
                        dq.price_to_book_ratio < qhr.maximum_price_to_book_ratio &&
                        //目前淨值比大於歷史最低淨值比
                        dq.price_to_book_ratio > qhr.minimum_price_to_book_ratio
                    {
                        return Ok(());
                    }

                    if dq.highest_price >= qhr.maximum_price {
                        qhr.maximum_price_date_on = dq.date;
                        qhr.maximum_price = dq.highest_price;
                    }

                    if dq.lowest_price <= qhr.minimum_price || qhr.minimum_price == Decimal::ZERO {
                        qhr.minimum_price_date_on = dq.date;
                        qhr.minimum_price = dq.lowest_price
                    }

                    if dq.price_to_book_ratio >= qhr.maximum_price_to_book_ratio {
                        qhr.maximum_price_to_book_ratio_date_on = dq.date;
                        qhr.maximum_price_to_book_ratio = dq.price_to_book_ratio;
                    }

                    if dq.price_to_book_ratio <= qhr.minimum_price_to_book_ratio
                        || qhr.minimum_price_to_book_ratio == Decimal::ZERO
                    {
                        qhr.minimum_price_to_book_ratio_date_on = dq.date;
                        qhr.minimum_price_to_book_ratio = dq.price_to_book_ratio
                    }

                   // qhr.upsert().await?;
                    //quote_history_records_guard.insert(qhr.security_code,qhr);
                }
            }
        }
        Err(why) => {
            return Err(anyhow!(
                "Failed to read quote_history_records cache because {:?}",
                why
            ));
        }
    }
    //logging::debug_file_async(format!("dq:{:#?}", dq));

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_calculate_moving_average() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 calculate_moving_average".to_string());
        let date = NaiveDate::from_ymd_opt(2023, 8, 1);
        match calculate_moving_average(date.unwrap()).await {
            Ok(_) => {
                logging::debug_file_async("calculate_moving_average() 完成".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to calculate_moving_average because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 calculate_moving_average".to_string());
    }
}
