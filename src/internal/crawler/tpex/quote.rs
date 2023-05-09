use crate::internal::crawler::tpex;
use crate::internal::{
    cache,
    cache::{TtlCacheInner, TTL},
    database::{model::daily_quote, model::daily_quote::FromWithExchange},
    logging, util, StockExchange,
};
use chrono::{DateTime, Datelike, Local};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use std::collections::HashMap;

// QuoteResponse 上櫃公司每日收盤資訊
#[derive(Debug, Deserialize)]
struct QuoteResponse {
    //pub report_date: String,
    #[serde(rename = "aaData")]
    pub aa_data: Vec<Vec<String>>,
    // i_total_records: i32,
}

// PeRatioAnalysis 上櫃股票個股本益比、殖利率、股價淨值比
#[derive(Debug, Deserialize)]
struct PeRatioAnalysisResponse {
    //pub date: String,
    #[serde(rename = "SecuritiesCompanyCode")]
    pub security_code: String,
    // company_name: String,
    // 本益比
    #[serde(rename = "PriceEarningRatio")]
    pub price_earning_ratio: String,
    // dividend_per_share: String,
    // 殖利率
    // yield_ratio: String,
    // 股價淨值比
    // price_book_ratio: String,
}

/// 抓取上櫃公司每日收盤資訊
pub async fn visit(date: DateTime<Local>) -> Option<Vec<daily_quote::Entity>> {
    let date_str = date.format("%Y%m%d").to_string();
    let pe_ratio_url = "https://www.tpex.org.tw/openapi/v1/tpex_mainboard_peratio_analysis";

    logging::info_file_async(format!("visit url:{}", pe_ratio_url));

    // 本益比
    let pe_ratio_response = match util::http::request_get_use_json::<Vec<PeRatioAnalysisResponse>>(
        pe_ratio_url,
    )
    .await
    {
        Ok(r) => r,
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to request_get_use_json({}) because {:?}",
                pe_ratio_url, why
            ));
            return None;
        }
    };

    let mut pe_ratio_analysis: HashMap<String, PeRatioAnalysisResponse> =
        HashMap::with_capacity(pe_ratio_response.len());
    for item in pe_ratio_response {
        pe_ratio_analysis.insert(item.security_code.to_string(), item);
    }

    let republic_date = date.year() - 1911;
    //https://www.tpex.org.tw/web/stock/aftertrading/daily_close_quotes/stk_quote_result.php?l=zh-tw&_=1681801169006
    let quote_url = format!(
        "{}/web/stock/aftertrading/otc_quotes_no1430/stk_wn1430_result.php?l=zh-tw&d={}{}&se=EW&_={}",
        tpex::HOST,
        republic_date,
        date.format("/%m/%d"),
        date.timestamp_millis()
    );

    logging::info_file_async(format!("visit url:{}", quote_url));

    let quote_response = match util::http::request_get_use_json::<QuoteResponse>(&quote_url).await {
        Ok(r) => r,
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to request_get_use_json({}) because {:?}",
                quote_url, why
            ));
            return None;
        }
    };

    let mut dqs: Vec<daily_quote::Entity> = Vec::with_capacity(2048);

    for item in quote_response.aa_data {
        let mut dq = daily_quote::Entity::from_with_exchange(StockExchange::TPEx, &item);
        //logging::debug_file_async(format!("item:{:?}", item));

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
                        dq.change_range =
                            (dq.closing_price - ldg.closing_price) / ldg.closing_price * dec!(100);
                    } else {
                        dq.change_range = dq.change / dq.opening_price * dec!(100);
                    }
                }
            }
        }

        if let Some(pe_ratio_analysis_response) = pe_ratio_analysis.get(&dq.security_code) {
            dq.price_earning_ratio = pe_ratio_analysis_response
                .price_earning_ratio
                .parse::<Decimal>()
                .unwrap_or_default()
        }

        dq.date = date.date_naive();
        dq.year = date.year();
        dq.month = date.month();
        dq.day = date.day();
        dq.record_time = date;
        dq.create_time = Local::now();

        dqs.push(dq);
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
        // now -= Duration::days(3);

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
