use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use hashbrown::HashMap;
use lazy_static::lazy_static;
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
        http::{self},
        map::Keyable,
        text,
    },
};

const UNSET_DATE: &str = "-";

lazy_static! {
    static ref PERIOD_RE: Regex = Regex::new(r"(\d+)([A-Z]\d)").unwrap();
}

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
        "https://{}/tw/StockDividendSchedule.asp?STOCK_ID={}&STEP=DATA",
        HOST, stock_symbol,
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
    let selector_td = Selector::parse("td").expect("Failed to parse td selector");

    let mut last_year: i32 = 0;
    let result: Result<Vec<GoodInfoDividend>, _> = document
        .select(&selector)
        .filter_map(|element| {
            //let tds: Vec<&str> = element.text().collect();
            let tds: Vec<_> = element
                .select(&selector_td)
                .map(|td| td.text().collect::<String>().trim().to_string())
                .collect();

            if tds.len() != 19 {
                return None;
            }

            let mut e = GoodInfoDividend::new(stock_symbol.to_string());

            let year_str = tds[0].to_string();
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
            let quarter = tds[1].to_string();
            match PERIOD_RE.captures(&quarter.to_uppercase()) {
                Some(caps) => {
                    e.year_of_dividend = parse_i32_safe(&caps[1], None)?;
                    e.quarter = caps.get(2)?.as_str().to_string();
                }
                None => {
                    e.year_of_dividend = parse_i32_safe(&quarter, Some(vec!['全', '年']))?;
                }
            }

            e.sum = text::parse_decimal(&tds[18], None).unwrap_or(Decimal::ZERO);

            if e.sum == Decimal::ZERO {
                return None;
            }

            e.earnings_cash = parse_decimal_safe(&tds[12]);
            e.capital_reserve_cash = parse_decimal_safe(&tds[13]);
            e.cash_dividend = parse_decimal_safe(&tds[14]);
            e.earnings_stock = parse_decimal_safe(&tds[15]);
            e.capital_reserve_stock = parse_decimal_safe(&tds[16]);
            e.stock_dividend = parse_decimal_safe(&tds[17]);

            if e.cash_dividend == Decimal::ZERO {
                e.ex_dividend_date1 = UNSET_DATE.to_string();
                e.payable_date1 = UNSET_DATE.to_string();
            } else if !tds[3].is_empty()  {
                e.ex_dividend_date1 = convert_date(&tds[3]).unwrap_or(UNSET_DATE.to_string());
                e.payable_date1 = convert_date(&tds[7]).unwrap_or(UNSET_DATE.to_string());
            }

            if e.stock_dividend == Decimal::ZERO {
                e.ex_dividend_date2 = UNSET_DATE.to_string();
                e.payable_date2 = UNSET_DATE.to_string();
            } else if !tds[8].is_empty() {
                e.ex_dividend_date2 = convert_date(&tds[8]).unwrap_or(UNSET_DATE.to_string());
            }

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

fn convert_date(s: &str) -> Option<String> {
    // 去除開頭的 '
    let trimmed = s.trim_start_matches('\'');

    // 拆解成 [yy, mm, dd]
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() != 3 {
        return None;
    }

    let yy: u32 = parts[0].parse().ok()?;
    let mm: u32 = parts[1].parse().ok()?;
    let dd: u32 = parts[2].parse().ok()?;

    // 決定年份
    let full_year = if yy < 50 { 2000 + yy } else { 1900 + yy };

    // 建立日期
    let date = NaiveDate::from_ymd_opt(full_year as i32, mm, dd)?;
    Some(date.format("%Y-%m-%d").to_string())
}

fn parse_decimal_safe(s: &str) -> Decimal {
    text::parse_decimal(s, None).unwrap_or(Decimal::ZERO)
}

fn parse_i32_safe(s: &str, strip: Option<Vec<char>>) -> Option<i32> {
    text::parse_i32(s, strip).ok()
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
