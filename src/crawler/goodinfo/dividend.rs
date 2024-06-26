use anyhow::{anyhow, Result};
use hashbrown::HashMap;
use regex::Regex;
use reqwest::header::{HeaderMap, COOKIE};
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use urlencoding::encode;

use crate::cache::SHARE;
use crate::{
    crawler::goodinfo::HOST,
    logging,
    util::{
        http::{self, element},
        map::Keyable,
        text,
    },
};

const UNSET_DATE: &str = "-";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoodInfoDividend {
    /// Security code
    pub stock_symbol: String,
    /// 盈餘現金股利 (Cash Dividend)
    pub earnings_cash: Decimal,
    /// 公積現金股利 (Capital Reserve)
    pub capital_reserve_cash: Decimal,
    /// 現金股利合計
    pub cash_dividend: Decimal,
    /// 盈餘股票股利 (Stock Dividend)
    pub earnings_stock: Decimal,
    /// 公積股票股利 (Capital Reserve)
    pub capital_reserve_stock: Decimal,
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

    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
}

impl GoodInfoDividend {
    pub fn new(stock_symbol: String) -> Self {
        GoodInfoDividend {
            quarter: "".to_string(),
            stock_symbol,
            earnings_cash: Default::default(),
            capital_reserve_cash: Default::default(),
            cash_dividend: Default::default(),
            earnings_stock: Default::default(),
            capital_reserve_stock: Default::default(),
            year: 0,
            ex_dividend_date1: "尚未公布".to_string(),
            ex_dividend_date2: "尚未公布".to_string(),
            payable_date1: "尚未公布".to_string(),
            payable_date2: "尚未公布".to_string(),
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

impl Keyable for GoodInfoDividend {
    fn key(&self) -> String {
        format!(
            "{}-{}-{}",
            self.stock_symbol, self.year_of_dividend, self.quarter
        )
    }

    fn key_with_prefix(&self) -> String {
        format!("GoodInfoDividend:{}", self.key())
    }
}

/// 抓取年度股利資料
pub async fn visit(stock_symbol: &str) -> Result<HashMap<i32, Vec<GoodInfoDividend>>> {
    let url = format!(
        "https://{}/tw/StockDividendPolicy.asp?STOCK_ID={}&STEP=DATA&SHEET={}&INITIALIZED=T",
        HOST,
        stock_symbol,
        encode("股利所屬年度")
    );

    let ua = http::user_agent::gen_random_ua();
    let mut headers = HeaderMap::new();

    /*headers.insert("Host", HOST.parse()?);
    headers.insert("Referer", url.parse()?);
    headers.insert("User-Agent", ua.parse()?);
    headers.insert(COOKIE,"CLIENT%5FID=20240517225034945%5F1%2E171%2E137%2E180".parse()?);
    //StockDividendPolicy.asp?STOCK_ID=2880
    //Lib.js/Initial.asp
    //Lib.js/Utility.asp
    //Lib.js/Cookie.asp
    let cookie_url = format!("https://{}/tw/StockDividendPolicy.asp?STOCK_ID=2880", HOST);
    let res = http::get_response(&cookie_url, Some(headers)).await?;
    let cookie =http::extract_cookies(&res);
    dbg!(&res);
    let t = &res.text().await?;
    dbg!(t);
    dbg!(cookie);

    headers = HeaderMap::new();*/

    headers.insert("Host", HOST.parse()?);
    headers.insert("Referer", url.parse()?);
    headers.insert("User-Agent", ua.parse()?);
    headers.insert("content-length", "0".parse()?);
    headers.insert("content-type", "application/x-www-form-urlencoded".parse()?);
    let cookie_val = format!("CLIENT%5FID=1st%5F{}; SL_G_WPT_TO=zh-TW; TW_STOCK_BROWSE_LIST={}; SL_GWPT_Show_Hide_tmp=1; SL_wptGlobTipTmp=1; IS_TOUCH_DEVICE=F; SCREEN_SIZE=WIDTH=2560&HEIGHT=1440",
                              encode(SHARE.get_current_ip().unwrap().as_str()),
                             stock_symbol);
    headers.insert(COOKIE, cookie_val.parse()?);

    let text = http::post(&url, Some(headers), None).await?;

    if text.contains("您的瀏覽量異常") {
        return Err(anyhow!("{} 瀏覽量異常", url));
    }

    if text.contains("初始化中") {
        return Err(anyhow!("{} 初始化中", url));
    }

    let document = Html::parse_document(text.as_str());
    let selector = Selector::parse("#tblDetail > tbody > tr")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let mut last_year: i32 = 0;
    let result: Result<Vec<GoodInfoDividend>, _> = document
        .select(&selector)
        .filter_map(|element| {
            let tds: Vec<&str> = element.text().collect();

            if tds.len() != 50 {
                return None;
            }

            //logging::debug_file_async(format!("tds({}):{:#?}",tds.len(), tds));
            let mut e = GoodInfoDividend::new(stock_symbol.to_string());
            //#tblDetail > tbody > tr:nth-child(5) > td:nth-child(2) > nobr > b
            let year_str = element::parse_value(&element, "td:nth-child(2) > nobr > b")?; //tds[1];
            if year_str.is_empty() {
                return None;
            }

            //股利發放年度
            e.year = match year_str.parse::<i32>() {
                Ok(y) => {
                    last_year = y;
                    y
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to i32::parse because(year:{}) {:#?}",
                        year_str, why
                    ));

                    if last_year == 0 {
                        return None;
                    }

                    last_year
                }
            };

            //股利所屬期間
            let quarter = element::parse_value(&element, "td:nth-child(21)")?;
            match Regex::new(r"(\d+)([A-Z]\d)") {
                Ok(re) => match re.captures(&quarter.to_uppercase()) {
                    None => {
                        // 2023
                        let year_of_dividend = text::parse_i32(&quarter, None).ok()?;
                        e.year_of_dividend = year_of_dividend;
                    }
                    Some(caps) => {
                        // 23Q1
                        let year_of_dividend = text::parse_i32(&caps[1], None).ok()?;
                        e.year_of_dividend = year_of_dividend + 2000;
                        e.quarter = caps.get(2)?.as_str().to_string();
                    }
                },
                Err(why) => {
                    logging::error_file_async(format!("Failed to Regex::new because {:#?}", why));
                    return None;
                }
            }
            e.sum = element::parse_to_decimal(&element, "td:nth-child(9)");

            if e.sum == Decimal::ZERO {
                return None;
            }

            e.earnings_cash = element::parse_to_decimal(&element, "td:nth-child(3)");
            e.capital_reserve_cash = element::parse_to_decimal(&element, "td:nth-child(4)");
            e.cash_dividend = element::parse_to_decimal(&element, "td:nth-child(5)");
            e.earnings_stock = element::parse_to_decimal(&element, "td:nth-child(6)");
            e.capital_reserve_stock = element::parse_to_decimal(&element, "td:nth-child(7)");
            e.stock_dividend = element::parse_to_decimal(&element, "td:nth-child(8)");
            e.earnings_per_share = element::parse_to_decimal(&element, "td:nth-child(22)");
            e.payout_ratio_cash = element::parse_to_decimal(&element, "td:nth-child(23)");
            e.payout_ratio_stock = element::parse_to_decimal(&element, "td:nth-child(24)");
            e.payout_ratio = element::parse_to_decimal(&element, "td:nth-child(25)");

            Some(Ok(e))
        })
        .collect();

    let result: Result<HashMap<i32, Vec<GoodInfoDividend>>, _> = result.map(|dividends| {
        let mut hashmap = HashMap::new();
        for dividend in dividends {
            hashmap
                .entry(dividend.year_of_dividend)
                .or_insert_with(Vec::new)
                .push(dividend);
        }

        for dividends in hashmap.values_mut() {
            dividends.iter_mut().for_each(|dividend| {
                // 如果是全年度配息(季配或半年配的總計，無需有配息日)或者配息金額為 0 時直接給 - 表示不用再抓取除息日
                if dividend.quarter.is_empty() || dividend.sum == Decimal::ZERO {
                    dividend.ex_dividend_date1 = UNSET_DATE.to_string();
                    dividend.ex_dividend_date2 = UNSET_DATE.to_string();
                    dividend.payable_date1 = UNSET_DATE.to_string();
                    dividend.payable_date2 = UNSET_DATE.to_string();
                }
            });
        }

        hashmap
    });

    result
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("dividend : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
