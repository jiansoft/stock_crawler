use crate::internal::cache::{TtlCacheInner, TTL};
use crate::internal::database::model::daily_quote::FromWithExchange;
use crate::internal::StockExchange;
use crate::{
    internal::cache, internal::database::model::daily_quote, internal::util::http, logging,
};
use chrono::{DateTime, Datelike, Local};
use core::result::Result::Ok;
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct ListedResponse {
    pub stat: Option<String>,
    pub data9: Option<Vec<Vec<String>>>,
}

async fn build_headers() -> HeaderMap {
    let mut h = HeaderMap::with_capacity(4);
    h.insert("Host", "www.twse.com.tw".parse().unwrap());
    h.insert(
        "Referer",
        "https://www.twse.com.tw/zh/page/trading/exchange/MI_INDEX.html"
            .parse()
            .unwrap(),
    );
    h.insert("X-Requested-With", "XMLHttpRequest".parse().unwrap());
    h.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.5615.50 Safari/537.36".parse().unwrap());
    h
}

/// 抓取上市公司每日收盤資訊
pub async fn visit(date: DateTime<Local>) -> Option<Vec<daily_quote::Entity>> {
    let date_str = date.format("%Y%m%d").to_string();
    let url = format!(
        "https://www.twse.com.tw/exchangeReport/MI_INDEX?response=json&date={}&type=ALLBUT0999&_={}",
        date_str,
        date.to_rfc3339()
    );

    logging::info_file_async(format!("visit url:{}", url,));

    let headers = build_headers().await;
    let data =
        match http::request_post_use_json::<http::Empty, ListedResponse>(&url, Some(headers), None)
            .await
        {
            Ok(data) => data,
            Err(e) => {
                logging::error_file_async(format!("Failed to fetch data from {}: {:?}", url, e));
                return None;
            }
        };

    let mut dqs = Vec::with_capacity(2048);

    if let Some(twse_dqs) = &data.data9 {
        for item in twse_dqs {
            //logging::debug_file_async(format!("item:{:?}", item));
            let mut dq = daily_quote::Entity::from_with_exchange(StockExchange::TWSE, item);

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
            dq.month = date.month();
            dq.day = date.day();
            dq.record_time = date;
            dq.create_time = Local::now();
            dqs.push(dq);
        }
    }

    Some(dqs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::SHARE;
    use chrono::{Duration, Timelike};

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
            None => {
                logging::debug_file_async(
                    "Failed to visit because response is no data".to_string(),
                );
            }
            Some(list) => logging::debug_file_async(format!("data({}):{:#?}", list.len(), list)),
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
