use crate::internal::{crawler::goodinfo::HOST, logging, util::http, util::text};
use anyhow::*;
use core::result::Result::Ok;
use regex::Regex;
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dividend {
    /// Security code
    pub stock_symbol: String,
    /// 盈餘現金股利 (Cash Dividend)
    pub earnings_cash_dividend: Decimal,
    /// 公積現金股利 (Capital Reserve)
    pub capital_reserve_cash_dividend: Decimal,
    /// 現金股利合計
    pub cash_dividend: Decimal,
    /// 盈餘股票股利 (Stock Dividend)
    pub earnings_stock_dividend: Decimal,
    /// 公積股票股利 (Capital Reserve)
    pub capital_reserve_stock_dividend: Decimal,
    /// 股票股利合計
    pub stock_dividend: Decimal,
    /// 股利合計 (Total Dividends)
    pub sum: Decimal,
    /// EPS
    pub earnings_per_share: Decimal,
    /// 盈餘分配率_配息(%)
    pub payout_ratio_cash: Decimal,
    /// 盈餘分配率_配股(%)
    pub payout_ratio_stock: Decimal,
    /// 盈餘分配率(%)
    pub payout_ratio: Decimal,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 發放季度 空字串:全年度 Q1~Q4:第一季~第四季 H1~H2︰上半季~下半季
    pub quarter: String,
    /// 發放年度 (Year)
    pub year: i32,
}

impl Dividend {
    pub fn new(stock_symbol: String) -> Self {
        Dividend {
            quarter: "".to_string(),
            stock_symbol,
            earnings_cash_dividend: Default::default(),
            capital_reserve_cash_dividend: Default::default(),
            cash_dividend: Default::default(),
            earnings_stock_dividend: Default::default(),
            capital_reserve_stock_dividend: Default::default(),
            year: 0,
            sum: Default::default(),
            earnings_per_share: Default::default(),
            payout_ratio_cash: Default::default(),
            payout_ratio_stock: Default::default(),
            payout_ratio: Default::default(),
            stock_dividend: Default::default(),
            year_of_dividend: 0,
        }
    }
}

/// 抓取年度股利資料
pub async fn visit(stock_symbol: &str) -> Result<Vec<Dividend>> {
    let url = format!(
        "https://{}/tw/StockDividendPolicy.asp?STOCK_ID={}",
        HOST, stock_symbol
    );

    logging::info_file_async(format!("visit url:{}", url,));

    let mut headers = HeaderMap::new();
    headers.insert("Host", HOST.parse()?);
    headers.insert("Referer", url.parse()?);
    headers.insert("X-Requested-With", "XMLHttpRequest".parse()?);
    headers.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.5615.50 Safari/537.36".parse()?);

    let text = http::request_get(&url, Some(headers)).await?;
    if text.contains("您的瀏覽量異常") {
        return Err(anyhow!("{} 瀏覽量異常", url));
    }

    let document = Html::parse_document(text.as_str());
    let selector = Selector::parse("#tblDetail > tbody > tr")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let mut year_index: i32 = 0;
    let result: Result<Vec<Dividend>, _> = document
        .select(&selector)
        .filter_map(|element| {
            //let tds: Vec<&str> = element.text().map(str::trim).collect();
            //logging::debug_file_async(format!("tds:{:#?}", tds));
            let mut e = Dividend::new(stock_symbol.to_string());
            let year_str = http::parse::element_value(&element, "td:nth-child(1)")?;
            e.year = match year_str.parse::<i32>() {
                Ok(y) => {
                    year_index = y;
                    e.year_of_dividend = y - 1;
                    y
                }
                Err(_) => {
                    let quarter = http::parse::element_value(&element, "td:nth-child(20)")?;

                    match Regex::new(r"(\d+)([A-Z]\d)") {
                        Ok(re) => match re.captures(&quarter.to_uppercase()) {
                            None => {
                                let year_of_dividend = text::parse_i32(&quarter, None).ok()?;
                                e.year_of_dividend = year_of_dividend;
                            }
                            Some(caps) => {
                                let year_of_dividend = text::parse_i32(&caps[1], None).ok()?;
                                e.year_of_dividend = year_of_dividend + 2000;
                                e.quarter = caps.get(2)?.as_str().to_string();
                            }
                        },
                        Err(why) => {
                            logging::error_file_async(format!(
                                "Failed to Regex::new because {:#?}",
                                why
                            ));
                            let year_of_dividend = text::parse_i32(&quarter, None).ok()?;
                            e.year_of_dividend = year_of_dividend;
                        }
                    }

                    year_index
                }
            };

            e.earnings_cash_dividend =
                http::parse::element_value_to_decimal(&element, "td:nth-child(2)");
            e.capital_reserve_cash_dividend =
                http::parse::element_value_to_decimal(&element, "td:nth-child(3)");
            e.cash_dividend = http::parse::element_value_to_decimal(&element, "td:nth-child(4)");
            e.earnings_stock_dividend =
                http::parse::element_value_to_decimal(&element, "td:nth-child(5)");
            e.capital_reserve_stock_dividend =
                http::parse::element_value_to_decimal(&element, "td:nth-child(6)");
            e.stock_dividend = http::parse::element_value_to_decimal(&element, "td:nth-child(7)");
            e.sum = http::parse::element_value_to_decimal(&element, "td:nth-child(8)");
            e.earnings_per_share =
                http::parse::element_value_to_decimal(&element, "td:nth-child(21)");
            e.payout_ratio_cash =
                http::parse::element_value_to_decimal(&element, "td:nth-child(22)");
            e.payout_ratio_stock =
                http::parse::element_value_to_decimal(&element, "td:nth-child(23)");
            e.payout_ratio = http::parse::element_value_to_decimal(&element, "td:nth-child(24)");

            Some(Ok(e))
        })
        .collect();

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("6613").await {
            Ok(e) => {
                logging::debug_file_async(format!("dividend : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
