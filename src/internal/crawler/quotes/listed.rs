use crate::{
    logging,
    internal::util::http,
    internal::cache_share,
    internal::database::model
};
use chrono::{Datelike, DateTime, Local};
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct ListedResponse {
    pub stat: Option<String>,
    pub data9: Option<Vec<Vec<String>>>,
}

/// 抓取上市公司每日收盤資訊
pub async fn retrieve(date: DateTime<Local>) {
    let date_str = date.format("%Y%m%d").to_string();
    let url = format!(
        "https://www.twse.com.tw/exchangeReport/MI_INDEX?response=json&date={}&type=ALLBUT0999&_={}",
        date_str,
        date.to_rfc3339()
    );

    logging::info_file_async(format!("visit url:{}", url,));

    let mut headers = HeaderMap::new();
    headers.insert("Host", "www.twse.com.tw".parse().unwrap());
    headers.insert(
        "Referer",
        "https://www.twse.com.tw/zh/page/trading/exchange/MI_INDEX.html"
            .parse()
            .unwrap(),
    );
    headers.insert("X-Requested-With", "XMLHttpRequest".parse().unwrap());
    headers.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.5615.50 Safari/537.36".parse().unwrap());

    let data =
        match http::request_post::<http::Empty, ListedResponse>(&url, Some(headers), None).await {
            Ok(response) => {
                // logging::info_file_async(format!("_response: {:?}", response));
                response
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to do_request_post_with_json because: {:?}",
                    why
                ));
                return;
            }
        };

    if let Some(dqs) = &data.data9 {
        for val in dqs {
            let mut dq = parse_and_create_daily_quote(val).await;
           // logging::info_file_async(format!("dq: {:?}", dq));
            if dq.closing_price == Decimal::ZERO
                && dq.highest_price == Decimal::ZERO
                && dq.lowest_price == Decimal::ZERO
                && dq.opening_price == Decimal::ZERO
            {
                continue;
            }

            if dq.change != Decimal::ZERO {
                if let Ok(last_trading_day_quotes) =
                    cache_share::CACHE_SHARE.last_trading_day_quotes.read()
                {
                    if let Some(ldg) = last_trading_day_quotes.get(&dq.security_code) {
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
            dq.month = date.month();
            dq.day = date.day();
            dq.record_time = date;
            dq.create_time = Local::now();
            match dq.upsert().await {
                Ok(_) => {}
                Err(why) => {
                    logging::error_file_async(format!("Failed to daily_quote.upsert() because {:?}", why));
                }
            }
        }
    }
}

async fn parse_and_create_daily_quote(val: &[String]) -> model::daily_quote::Entity {
    let mut dq = model::daily_quote::Entity::new(val[0].to_string());
    let decimal_fields = [
        (2, &mut dq.trading_volume),
        (3, &mut dq.transaction),
        (4, &mut dq.trade_value),
        (5, &mut dq.opening_price),
        (6, &mut dq.highest_price),
        (7, &mut dq.lowest_price),
        (8, &mut dq.closing_price),
        (10, &mut dq.change),
        (11, &mut dq.last_best_bid_price),
        (12, &mut dq.last_best_bid_volume),
        (13, &mut dq.last_best_ask_price),
        (14, &mut dq.last_best_ask_volume),
        (15, &mut dq.price_earning_ratio),
    ];

    for (index, field) in decimal_fields {
        let d = val[index].replace(',', "");
        *field = d.parse::<Decimal>().unwrap_or_default();
    }

    if val[9].contains('-') {
        dq.change = -dq.change;
    }

    dq.create_time = Local::now();
    let default_date = DateTime::parse_from_rfc3339("1970-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Local);
    dq.maximum_price_in_year_date_on = default_date;
    dq.minimum_price_in_year_date_on = default_date;

    dq
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache_share::CACHE_SHARE;
    use chrono::{Duration, Timelike};

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        let mut now = Local::now();
        if now.hour() < 15 {
            now -= Duration::days(1);
        }

        retrieve(now).await;
    }
}
