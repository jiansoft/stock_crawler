use anyhow::Result;
use chrono::{DateTime, Datelike, Local};
use hashbrown::HashMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;

use crate::{
    cache::{self, TtlCacheInner, TTL},
    crawler::tpex,
    database::{table, table::daily_quote::FromWithExchange},
    internal::StockExchange,
    util,
};

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
pub async fn visit(date: DateTime<Local>) -> Result<Vec<table::daily_quote::DailyQuote>> {
    let date_str = date.format("%Y%m%d").to_string();
    let pe_ratio_url = "https://www.tpex.org.tw/openapi/v1/tpex_mainboard_peratio_analysis";
    // 本益比
    let pe_ratio_response =
        util::http::get_use_json::<Vec<PeRatioAnalysisResponse>>(pe_ratio_url).await?;
    let mut pe_ratio_analysis: HashMap<String, PeRatioAnalysisResponse> =
        HashMap::with_capacity(pe_ratio_response.len());

    for item in pe_ratio_response {
        pe_ratio_analysis.insert(item.security_code.to_string(), item);
    }

    let republic_date = date.year() - 1911;
    //https://www.tpex.org.tw/web/stock/aftertrading/daily_close_quotes/stk_quote_result.php?l=zh-tw&_=1681801169006
    let quote_url = format!(
        "https://{}/web/stock/aftertrading/otc_quotes_no1430/stk_wn1430_result.php?l=zh-tw&d={}{}&se=EW&_={}",
        tpex::HOST,
        republic_date,
        date.format("/%m/%d"),
        date.timestamp_millis()
    );

    let quote_response = util::http::get_use_json::<QuoteResponse>(&quote_url).await?;
    let mut dqs: Vec<table::daily_quote::DailyQuote> = Vec::with_capacity(2048);

    for item in quote_response.aa_data {
        let mut dq = table::daily_quote::DailyQuote::from_with_exchange(StockExchange::TPEx, &item);
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
        dq.month = date.month() as i32;
        dq.day = date.day() as i32;
        dq.record_time = date;
        dq.create_time = Local::now();

        dqs.push(dq);
    }

    Ok(dqs)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Timelike};

    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
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
