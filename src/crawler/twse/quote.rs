use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate, TimeZone};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{
    cache::{self, TtlCacheInner, TTL},
    crawler::twse,
    database::table::{self, daily_quote::FromWithExchange},
    declare::StockExchange,
    logging,
    util::{http, map::Keyable},
};

/*#[derive(Serialize, Deserialize, Debug)]
struct ListedResponse {
    pub stat: Option<String>,
    pub data9: Option<Vec<Vec<String>>>,
}*/

#[derive(Serialize, Deserialize, Debug)]
pub struct ListedResponse {
    pub stat: Option<String>,
    #[serde(rename = "tables")]
    pub tables: Vec<Table>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Table {
    #[serde(rename = "title")]
    pub title: Option<String>,

    #[serde(rename = "fields")]
    pub fields: Option<Vec<String>>,

    #[serde(rename = "data")]
    pub data: Option<Vec<Vec<String>>>,

    #[serde(rename = "hints")]
    pub hints: Option<String>,
}

/// 抓取上市公司每日收盤資訊
pub async fn visit(date: NaiveDate) -> Result<Vec<table::daily_quote::DailyQuote>> {
    let date_str = date.format("%Y%m%d").to_string();
    let url = format!(
        "https://www.{}/exchangeReport/MI_INDEX?response=json&date={}&type=ALLBUT0999&_={}",
        twse::HOST,
        date_str,
        date
    );

    //let headers = build_headers().await;
    let data = http::get_json::<ListedResponse>(&url).await?;
    let mut dqs = Vec::with_capacity(2048);
    if data.tables.len() >= 9 {
        if let Some(twse_dqs) = &data.tables[8].data {
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

                let daily_quote_memory_key = dq.key();

                if TTL.daily_quote_contains_key(&daily_quote_memory_key) {
                    continue;
                }

                if !dq.change.is_zero() {
                    if let Some(ldg) = cache::SHARE
                        .get_last_trading_day_quotes(&dq.security_code)
                        .await
                    {
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

                dq.date = date;
                dq.year = date.year();
                dq.month = date.month() as i32;
                dq.day = date.day() as i32;

                let record_time = date
                    .and_hms_opt(15, 0, 0)
                    .and_then(|naive| Local.from_local_datetime(&naive).single())
                    .unwrap_or_else(|| {
                        logging::warn_file_async("Failed to create DateTime<Local> from NaiveDateTime, using current time as default.".to_string());
                        Local::now()
                    });

                dq.record_time = record_time;
                dq.create_time = Local::now();
                dqs.push(dq);
            }
        }
    }
    Ok(dqs)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Timelike};
    use std::time::Duration;

    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let mut now = Local::now();
        if now.hour() < 15 {
            now -= TimeDelta::try_days(1).unwrap();
        }
        //now -= Duration::days(3);

        logging::debug_file_async("開始 visit".to_string());

        match visit(now.date_naive()).await {
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
            Ok(list) => {
                logging::debug_file_async(format!("data:{:#?}", list));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
