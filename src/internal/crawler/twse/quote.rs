use anyhow::Result;
use chrono::{DateTime, Datelike, Local};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::internal::{
    cache::{self, TtlCacheInner, TTL},
    crawler::twse,
    database::table::{self, daily_quote::FromWithExchange},
    logging,
    util::http,
    StockExchange,
};
use crate::internal::crawler::twse::build_headers;

#[derive(Serialize, Deserialize, Debug)]
struct ListedResponse {
    pub stat: Option<String>,
    pub data9: Option<Vec<Vec<String>>>,
}

/// 抓取上市公司每日收盤資訊
pub async fn visit(date: DateTime<Local>) -> Result<Vec<table::daily_quote::DailyQuote>> {
    let date_str = date.format("%Y%m%d").to_string();
    let url = format!(
        "https://www.{}/exchangeReport/MI_INDEX?response=json&date={}&type=ALLBUT0999&_={}",
        twse::HOST,
        date_str,
        date.to_rfc3339()
    );

    logging::info_file_async(format!("visit url:{}", url,));

    let headers = build_headers().await;
    let data =
        http::post_use_json::<http::Empty, ListedResponse>(&url, Some(headers), None).await?;
    let mut dqs = Vec::with_capacity(2048);

    if let Some(twse_dqs) = &data.data9 {
        for item in twse_dqs {
            //logging::debug_file_async(format!("item:{:?}", item));
            let mut dq =
                table::daily_quote::DailyQuote::from_with_exchange(StockExchange::TWSE, item);

            if dq.closing_price.is_zero()
                && dq.highest_price.is_zero()
                && dq.lowest_price.is_zero()
                && dq.opening_price.is_zero()
            {
                continue;
            }

            let daily_quote_memory_key = format!("DailyQuote:{}-{}", date_str, dq.security_code);

            if TTL.daily_quote_contains_key(&daily_quote_memory_key) {
                continue;
            }

            if !dq.change.is_zero() {
                if let Ok(ltdq) = cache::SHARE.last_trading_day_quotes.read() {
                    if let Some(ldg) = ltdq.get(&dq.security_code) {
                        if ldg.closing_price > Decimal::ZERO {
                            // 漲幅 = (现价-上一个交易日收盘价）/ 上一个交易日收盘价*100%
                            dq.change_range = (dq.closing_price - ldg.closing_price)
                                / ldg.closing_price
                                * dec!(100);
                        } else {
                            dq.change_range = dq.change / dq.opening_price * dec!(100);
                        }
                    }
                }
            }

            dq.date = date.date_naive();
            dq.year = date.year();
            dq.month = date.month() as i32;
            dq.day = date.day() as i32;
            dq.record_time = date;
            dq.create_time = Local::now();
            dqs.push(dq);
        }
    }

    Ok(dqs)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Timelike};

    use crate::internal::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let mut now = Local::now();
        if now.hour() < 15 {
            now -= Duration::days(1);
        }
        //now -= Duration::days(3);

        logging::debug_file_async("開始 visit".to_string());

        match visit(now).await {
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
            Ok(list) => {
                logging::debug_file_async(format!("data:{:#?}", list));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
