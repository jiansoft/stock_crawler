use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use futures::{stream, StreamExt};
use rust_decimal::Decimal;

use crate::internal::{
    cache::SHARE,
    database::table::{
        daily_quote, daily_quote::DailyQuote, quote_history_record::QuoteHistoryRecord,
    },
    logging, util,
};

/// 計算每家公司指定日期的均線值
pub async fn calculate_moving_average(date: NaiveDate) -> Result<()> {
    let quotes = daily_quote::fetch_daily_quotes_by_date(date).await?;
    stream::iter(quotes)
        .for_each_concurrent(util::concurrent_limit_32(), |dq| async move {
            if let Err(why) = process_daily_quote_moving_average(dq).await {
                logging::error_file_async(format!(
                    "Failed to moving_average::calculate because {:?}",
                    why
                ));
            }
        })
        .await;
    Ok(())
}

pub(crate) async fn process_daily_quote_moving_average(mut dq: DailyQuote) -> Result<()> {
    dq.fill_moving_average().await?;
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

    /*  if dq.security_code == "2330" {
        dbg!(&dq);
    }*/

    dq.update_moving_average().await?;

    let qhr = match SHARE.quote_history_records.write() {
        Ok(mut quote_history_records_guard) => {
            match quote_history_records_guard.get_mut(&dq.security_code) {
                None => {
                    let mut qhr = QuoteHistoryRecord::new(dq.security_code.to_string());
                    qhr.maximum_price_date_on = dq.date;
                    qhr.maximum_price_to_book_ratio_date_on = dq.date;
                    qhr.minimum_price_date_on = dq.date;
                    qhr.minimum_price_to_book_ratio_date_on = dq.date;
                    qhr.maximum_price = dq.highest_price;
                    qhr.minimum_price = dq.lowest_price;
                    qhr.maximum_price_to_book_ratio = dq.price_to_book_ratio;
                    qhr.minimum_price_to_book_ratio = dq.price_to_book_ratio;
                    qhr
                }
                Some(qhr) => {
                    let highest_price = dq.highest_price.round_dp(4);
                    let lowest_price = dq.lowest_price.round_dp(4);
                    let price_to_book_ratio = dq.price_to_book_ratio.round_dp(4);

                    let maximum_price = qhr.maximum_price.round_dp(4);
                    let minimum_price = qhr.minimum_price.round_dp(4);
                    let maximum_price_to_book_ratio = qhr.maximum_price_to_book_ratio.round_dp(4);
                    let minimum_price_to_book_ratio = qhr.minimum_price_to_book_ratio.round_dp(4);

                    //目前最高價小於歷史最高價
                    if (highest_price <= maximum_price &&
                        //目前最低價高於歷史最低價
                        lowest_price >= minimum_price &&
                        //目前淨值比小於歷史最高淨值比
                        price_to_book_ratio <= maximum_price_to_book_ratio &&
                        //目前淨值比大於歷史最低淨值比
                        price_to_book_ratio >= minimum_price_to_book_ratio)
                        || (price_to_book_ratio == Decimal::ZERO
                            && minimum_price_to_book_ratio == Decimal::ZERO)
                    {
                        return Ok(());
                    }

                    if highest_price > maximum_price {
                        qhr.maximum_price_date_on = dq.date;
                        qhr.maximum_price = highest_price;
                    }

                    if lowest_price < minimum_price || minimum_price == Decimal::ZERO {
                        qhr.minimum_price_date_on = dq.date;
                        qhr.minimum_price = lowest_price
                    }

                    if price_to_book_ratio > maximum_price_to_book_ratio {
                        qhr.maximum_price_to_book_ratio_date_on = dq.date;
                        qhr.maximum_price_to_book_ratio = price_to_book_ratio;
                    }

                    if price_to_book_ratio < minimum_price_to_book_ratio
                        || minimum_price_to_book_ratio == Decimal::ZERO
                    {
                        qhr.minimum_price_to_book_ratio_date_on = dq.date;
                        qhr.minimum_price_to_book_ratio = price_to_book_ratio
                    }

                    qhr.clone()
                }
            }
        }
        Err(why) => {
            return Err(anyhow!(
                "Failed to read quote_history_records cache because {:?}",
                why
            ));
        }
    };

    if let Err(why) = qhr.upsert().await {
        logging::debug_file_async(format!(
            "Failed to quote_history_records::upsert() because:{:?}",
            why
        ));
    }

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
        let date = NaiveDate::from_ymd_opt(2023, 8, 2);
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
