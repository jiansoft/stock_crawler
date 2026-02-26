use crate::{
    cache::{self, TtlCacheInner, TTL},
    crawler::tpex,
    database::table::{self, daily_quote::FromWithExchange},
    declare::StockExchange,
    logging,
    util::{self, map::Keyable},
};
use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate, TimeZone};
use hashbrown::HashMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use serde_derive::Serialize;

// QuoteResponse 上櫃公司每日收盤資訊
/*#[derive(Debug, Deserialize)]
struct QuoteResponse {
    //pub report_date: String,
    #[serde(rename = "aaData")]
    pub aa_data: Vec<Vec<String>>,
    // i_total_records: i32,
}*/

// QuoteResponse 上櫃公司每日收盤資訊
#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteResponse {
    #[serde(rename = "tables")]
    pub tables: Vec<Table>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Table {
    #[serde(rename = "data")]
    pub data: Option<Vec<Vec<String>>>,
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
pub async fn visit(date: NaiveDate) -> Result<Vec<table::daily_quote::DailyQuote>> {
    let pe_ratio_url = "https://www.tpex.org.tw/openapi/v1/tpex_mainboard_peratio_analysis";
    // 本益比
    let pe_ratio_response =
        util::http::get_json::<Vec<PeRatioAnalysisResponse>>(pe_ratio_url).await?;
    let mut pe_ratio_analysis: HashMap<String, PeRatioAnalysisResponse> =
        HashMap::with_capacity(pe_ratio_response.len());

    for item in pe_ratio_response {
        pe_ratio_analysis.insert(item.security_code.to_string(), item);
    }

    let republic_date = util::datetime::gregorian_year_to_roc_year(date.year());
    //https://www.tpex.org.tw/web/stock/aftertrading/daily_close_quotes/stk_quote_result.php?l=zh-tw&_=1681801169006
    let quote_url = format!(
        "https://{}/web/stock/aftertrading/otc_quotes_no1430/stk_wn1430_result.php?l=zh-tw&d={}{}&se=EW&_={}",
        tpex::HOST,
        republic_date,
        date.format("/%m/%d"),
        date
    );

    let quote_response = util::http::get_json::<QuoteResponse>(&quote_url).await?;
    let mut dqs: Vec<table::daily_quote::DailyQuote> = Vec::with_capacity(2048);
    if !quote_response.tables.is_empty() {
        if let Some(tpex_dqs) = &quote_response.tables[0].data {
            for item in tpex_dqs {
                let mut dq =
                    table::daily_quote::DailyQuote::from_with_exchange(StockExchange::TPEx, item);
                //logging::debug_file_async(format!("item:{:?}", item));

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
                        .get_last_trading_day_quotes(&dq.stock_symbol)
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

                if let Some(pe_ratio_analysis_response) = pe_ratio_analysis.get(&dq.stock_symbol) {
                    dq.price_earning_ratio = pe_ratio_analysis_response
                        .price_earning_ratio
                        .parse::<Decimal>()
                        .unwrap_or_default()
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
        // now -= Duration::days(3);

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
