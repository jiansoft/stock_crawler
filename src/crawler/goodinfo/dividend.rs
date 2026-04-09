use anyhow::{anyhow, Result};
use chrono::{NaiveDate, Utc};
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

/// 依股利所屬年度由新到舊排序的 Goodinfo 股利資料。
///
/// tuple 的第一個 `i32` 是 `year_of_dividend`，代表「股利所屬年度」，
/// 不是實際發放年度 `year`。年份一律正規化為西元年，例如 Goodinfo 的
/// `25Q2` 會轉成 `2025`。第二個值是該所屬年度底下的股利明細。
pub type GoodInfoDividendsByYear = Vec<(i32, Vec<GoodInfoDividend>)>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Goodinfo 股利政策頁面的單筆股利資料。
///
/// 一筆資料可能代表全年、半年度或單季股利，因此 `quarter` 可能為
/// 空字串、`Q1~Q4` 或 `H1~H2`。
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
    /// 建立一筆帶預設值的 `GoodInfoDividend`。
    ///
    /// 日期欄位會先以「尚未公布」初始化，數值欄位則以 `0` 初始化，
    /// 再由解析流程逐欄覆寫。
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

/// 抓取 Goodinfo 股利資料，並補齊盈餘分配率。
///
/// Goodinfo 將股利資料拆在兩個 AJAX 端點：
///
/// - `StockDividendSchedule.asp`：提供除權息日期、發放日與股利金額。
/// - `StockDividendPolicy.asp?SHEET2=盈餘分配率`：提供 EPS、盈餘配息率、盈餘配股率與合計盈餘分配率。
///
/// 這個函式會先解析股利日程，再用 `stock_symbol + year_of_dividend + quarter`
/// 作為 key，把盈餘分配率合併回相同股利資料。Goodinfo 有些年度彙總列不一定存在於
/// 日程端點，因此只會合併已經存在於日程資料中的列，避免額外產生不完整的股利紀錄。
///
/// 回傳值會依股利所屬年度由新到舊排序。這裡不用 `HashMap`，因為 `HashMap`
/// 不保證迭代順序，無法表達穩定的 desc 排序。
pub async fn visit(stock_symbol: &str) -> Result<GoodInfoDividendsByYear> {
    let schedule_url = format!(
        "https://{}/tw/StockDividendSchedule.asp?STOCK_ID={}&STEP=DATA",
        HOST, stock_symbol,
    );
    let policy_url = format!(
        "https://{}/tw/StockDividendPolicy.asp?STEP=DATA&STOCK_ID={}&PRICE_ADJ=F&SHEET={}&SHEET2={}",
        HOST,
        stock_symbol,
        encode("股利發放年度"),
        encode("盈餘分配率"),
    );

    let ua = http::user_agent::gen_random_ua();

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

    let headers = build_headers(stock_symbol, schedule_url.as_str(), ua.as_str())?;

    let text = http::post(&schedule_url, Some(headers), None).await?;
    validate_response(schedule_url.as_str(), text.as_str())?;

    let mut dividends = parse_schedule_dividends(stock_symbol, text.as_str())?;

    let policy_referer = format!(
        "https://{}/tw/StockDividendPolicy.asp?STOCK_ID={}",
        HOST, stock_symbol
    );
    let headers = build_headers(stock_symbol, policy_referer.as_str(), ua.as_str())?;
    let text = http::post(&policy_url, Some(headers), None).await?;
    validate_response(policy_url.as_str(), text.as_str())?;

    let payout_ratios = parse_payout_ratio_dividends(stock_symbol, text.as_str())?;
    merge_payout_ratios(&mut dividends, payout_ratios);

    let result = dividends_to_year_groups(dividends);

    Ok(result)
}

/// 建立 Goodinfo 請求需要的 headers。
///
/// `Host`、`content-length` 與 `content-type` 交給 `reqwest` 自動處理；
/// 手動指定這些 header 可能讓 Goodinfo 的 IIS 回傳 `Bad Request`。
/// 這裡只補 referer、user-agent 與 Goodinfo 判斷瀏覽器狀態會用到的 cookie。
fn build_headers(stock_symbol: &str, referer: &str, user_agent: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();

    headers.insert("Referer", referer.parse()?);
    headers.insert("User-Agent", user_agent.parse()?);
    let cookie_val = format!(
        "CLIENT_KEY={}; CLIENT%5FID=1st%5F{}; SL_G_WPT_TO=zh-TW; TW_STOCK_BROWSE_LIST={}; SL_GWPT_Show_Hide_tmp=1; SL_wptGlobTipTmp=1; IS_TOUCH_DEVICE=F; SCREEN_SIZE=WIDTH=2560&HEIGHT=1440",
        client_key(),
        encode(SHARE.get_current_ip().unwrap().as_str()),
        stock_symbol
    );
    headers.insert(COOKIE, cookie_val.parse()?);

    Ok(headers)
}

/// 產生 Goodinfo 前端 JavaScript 會設定的 `CLIENT_KEY` cookie。
///
/// Goodinfo 會用這個 cookie 搭配螢幕尺寸與瀏覽清單判斷請求是否完成初始化。
/// 時區固定使用台灣時間 UTC+8，對應瀏覽器端 `GetTimezoneOffset()` 的 `-480`。
fn client_key() -> String {
    let timezone_offset = -480.0;
    let now_days = (Utc::now().timestamp_millis() as f64 / 86_400_000.0) - timezone_offset / 1440.0;
    format!(
        "2.2|44099.7780145202|46322.0002367424|-480|{}|{}",
        now_days, now_days
    )
}

/// 驗證 Goodinfo 回應是否可解析。
///
/// Goodinfo 在尚未初始化、流量異常或請求格式錯誤時仍可能回傳 200，
/// 所以不能只看 HTTP status。缺少 `tblDetail` 時直接回錯，避免採集流程靜默寫入空資料。
fn validate_response(url: &str, text: &str) -> Result<()> {
    if text.contains("您的瀏覽量異常") {
        return Err(anyhow!("{} 瀏覽量異常", url));
    }

    if text.contains("初始化中") {
        return Err(anyhow!("{} 初始化中", url));
    }

    if !text.contains("tblDetail") {
        return Err(anyhow!(
            "{} 缺少 tblDetail: {}",
            url,
            text::truncate(text, 200)
        ));
    }

    Ok(())
}

/// 解析 `StockDividendSchedule.asp` 的股利日程資料。
///
/// 此端點每列應有 19 個欄位，主要提供股利發放年度、股利所屬期間、
/// 現金/股票股利明細、除息日、除權日與現金股利發放日。這裡不解析盈餘分配率，
/// 因為該欄位位於 Goodinfo 的股利政策端點。
fn parse_schedule_dividends(stock_symbol: &str, text: &str) -> Result<Vec<GoodInfoDividend>> {
    let document = Html::parse_document(text);
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
                    e.year_of_dividend = normalize_goodinfo_year(parse_i32_safe(&caps[1], None)?);
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
            } else if !tds[3].is_empty() {
                e.ex_dividend_date1 = convert_date(&tds[3]).unwrap_or(UNSET_DATE.to_string());
                e.payable_date1 = convert_date(&tds[7]).unwrap_or(UNSET_DATE.to_string());
            }

            if e.stock_dividend == Decimal::ZERO {
                e.ex_dividend_date2 = UNSET_DATE.to_string();
                e.payable_date2 = UNSET_DATE.to_string();
            } else if !tds[8].is_empty() {
                e.ex_dividend_date2 = convert_date(&tds[8]).unwrap_or(UNSET_DATE.to_string());
            }

            // 修正：若為季配或半年配，year 應優先以發放日 (payable_date1) 為準，若無則以除息日為準
            if !e.quarter.is_empty() {
                let mut target_year = 0;

                // 優先從發放日提取年份
                if e.payable_date1 != "尚未公布" && e.payable_date1 != UNSET_DATE {
                    if let Some(y) = e
                        .payable_date1
                        .split('-')
                        .next()
                        .and_then(|s| s.parse::<i32>().ok())
                    {
                        target_year = y;
                    }
                }

                // 若無發放日，從除息日提取年份
                if target_year == 0
                    && e.ex_dividend_date1 != "尚未公布"
                    && e.ex_dividend_date1 != UNSET_DATE
                {
                    if let Some(y) = e
                        .ex_dividend_date1
                        .split('-')
                        .next()
                        .and_then(|s| s.parse::<i32>().ok())
                    {
                        target_year = y;
                    }
                }

                if target_year != 0 {
                    e.year = target_year;
                }
            }

            Some(Ok(e))
        })
        .collect();

    result
}

/// 解析 `StockDividendPolicy.asp?SHEET2=盈餘分配率` 的盈餘分配率資料。
///
/// 此端點每列應有 15 個欄位。第 12 欄為 EPS，第 13 至 15 欄依序為
/// 盈餘配息率、盈餘配股率與合計盈餘分配率。年度彙總列的股利所屬期間為 `-`，
/// 這類列不會合併回日程資料，因此在這裡直接略過。
fn parse_payout_ratio_dividends(stock_symbol: &str, text: &str) -> Result<Vec<GoodInfoDividend>> {
    let document = Html::parse_document(text);
    let selector = Selector::parse("#tblDetail > tbody > tr")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let selector_td = Selector::parse("td").expect("Failed to parse td selector");

    let mut last_year: i32 = 0;
    document
        .select(&selector)
        .filter_map(|element| {
            let tds: Vec<_> = element
                .select(&selector_td)
                .map(|td| td.text().collect::<String>().trim().to_string())
                .collect();

            if tds.len() != 15 {
                return None;
            }

            let mut e = GoodInfoDividend::new(stock_symbol.to_string());

            let year_str = tds[0].replace("&nbsp;", "").trim().to_string();
            if year_str.contains("累計") {
                return None;
            }

            e.year = match parse_i32_safe(&year_str, Some(vec!['∟'])) {
                Some(y) => {
                    last_year = y;
                    y
                }
                None => {
                    if last_year == 0 {
                        return None;
                    }

                    last_year
                }
            };

            let quarter = tds[1].to_string();
            if quarter == "-" {
                return None;
            }

            match PERIOD_RE.captures(&quarter.to_uppercase()) {
                Some(caps) => {
                    e.year_of_dividend = normalize_goodinfo_year(parse_i32_safe(&caps[1], None)?);
                    e.quarter = caps.get(2)?.as_str().to_string();
                }
                None => {
                    e.year_of_dividend = parse_i32_safe(&quarter, None)?;
                }
            }

            e.sum = parse_decimal_safe(&tds[8]);
            if e.sum == Decimal::ZERO {
                return None;
            }

            e.earnings_per_share = parse_decimal_safe(&tds[11]);
            e.payout_ratio_cash = parse_decimal_safe(&tds[12]);
            e.payout_ratio_stock = parse_decimal_safe(&tds[13]);
            e.payout_ratio = parse_decimal_safe(&tds[14]);

            Some(Ok(e))
        })
        .collect()
}

/// 將盈餘分配率資料合併回股利日程資料。
///
/// 合併 key 使用 `GoodInfoDividend::key()`，也就是
/// `stock_symbol-year_of_dividend-quarter`。只更新 EPS 與三個盈餘分配率欄位，
/// 不覆寫除權息日期與股利金額，避免不同 Goodinfo 端點的欄位語意互相污染。
fn merge_payout_ratios(dividends: &mut [GoodInfoDividend], payout_ratios: Vec<GoodInfoDividend>) {
    let payout_ratio_map: HashMap<_, _> = payout_ratios
        .into_iter()
        .map(|dividend| (dividend.key(), dividend))
        .collect();

    dividends.iter_mut().for_each(|dividend| {
        if let Some(payout_ratio) = payout_ratio_map.get(&dividend.key()) {
            dividend.earnings_per_share = payout_ratio.earnings_per_share;
            dividend.payout_ratio_cash = payout_ratio.payout_ratio_cash;
            dividend.payout_ratio_stock = payout_ratio.payout_ratio_stock;
            dividend.payout_ratio = payout_ratio.payout_ratio;
        }
    });
}

/// 將股利資料依股利所屬年度分組，並由新到舊排序。
///
/// 全年度或股利合計為 0 的資料不需要再追除權息日期，因此日期欄位統一設為 `-`。
fn dividends_to_year_groups(dividends: Vec<GoodInfoDividend>) -> GoodInfoDividendsByYear {
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

    let mut groups: Vec<_> = hashmap.into_iter().collect();
    groups.sort_by(|(left_year, _), (right_year, _)| right_year.cmp(left_year));
    groups
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

/// 將 Goodinfo 股利所屬期間中的年份正規化為西元年。
///
/// Goodinfo 的季度列會用二位數年份，例如 `25Q2`。這裡用 50 年作為分界：
/// `00..49` 視為 `2000..2049`，`50..99` 視為 `1950..1999`；已經是四位數的年份則原樣保留。
fn normalize_goodinfo_year(year: i32) -> i32 {
    match year {
        0..=49 => 2000 + year,
        50..=99 => 1900 + year,
        _ => year,
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_parse_and_merge_payout_ratio() {
        let schedule_html = r#"
            <table id='tblDetail'>
                <tbody>
                    <tr>
                        <td>2025</td><td>25Q2</td><td></td><td>'25/12/11</td><td>1,500</td>
                        <td>'25/12/11</td><td>1</td><td>'26/01/08</td><td></td><td></td><td></td><td></td>
                        <td>5</td><td>0</td><td>5</td><td>0</td><td>0</td><td>0</td><td>5</td>
                    </tr>
                </tbody>
            </table>
        "#;
        let payout_ratio_html = r#"
            <table id='tblDetail'>
                <tbody>
                    <tr>
                        <td>2025</td><td>-</td><td>19</td><td>0</td><td>19</td><td>0</td><td>0</td><td>0</td>
                        <td>19</td><td>-</td><td>-</td><td>56.3</td><td>33.7</td><td>0</td><td>33.7</td>
                    </tr>
                    <tr>
                        <td>&nbsp;&nbsp;&nbsp;&nbsp;∟12/11&nbsp;</td><td>25Q2</td><td>5</td><td>0</td><td>5</td>
                        <td>0</td><td>0</td><td>0</td><td>5</td><td>1</td><td>-</td><td>15.36</td><td>32.5</td><td>0</td><td>32.5</td>
                    </tr>
                </tbody>
            </table>
        "#;

        let mut dividends = parse_schedule_dividends("2330", schedule_html).unwrap();
        let payout_ratios = parse_payout_ratio_dividends("2330", payout_ratio_html).unwrap();

        merge_payout_ratios(&mut dividends, payout_ratios);

        assert_eq!(dividends.len(), 1);
        assert_eq!(dividends[0].year_of_dividend, 2025);
        assert_eq!(dividends[0].earnings_per_share, dec!(15.36));
        assert_eq!(dividends[0].payout_ratio_cash, dec!(32.5));
        assert_eq!(dividends[0].payout_ratio_stock, dec!(0));
        assert_eq!(dividends[0].payout_ratio, dec!(32.5));
    }

    #[test]
    fn test_normalize_goodinfo_year() {
        assert_eq!(normalize_goodinfo_year(25), 2025);
        assert_eq!(normalize_goodinfo_year(99), 1999);
        assert_eq!(normalize_goodinfo_year(2025), 2025);
    }

    #[test]
    fn test_dividends_to_year_groups_sorts_year_desc() {
        let mut dividend_2024 = GoodInfoDividend::new("2330".to_string());
        dividend_2024.year_of_dividend = 2024;
        dividend_2024.sum = dec!(1);

        let mut dividend_2026 = GoodInfoDividend::new("2330".to_string());
        dividend_2026.year_of_dividend = 2026;
        dividend_2026.sum = dec!(1);

        let mut dividend_2025 = GoodInfoDividend::new("2330".to_string());
        dividend_2025.year_of_dividend = 2025;
        dividend_2025.sum = dec!(1);

        let groups = dividends_to_year_groups(vec![dividend_2024, dividend_2026, dividend_2025]);
        let years: Vec<_> = groups.into_iter().map(|(year, _)| year).collect();

        assert_eq!(years, vec![2026, 2025, 2024]);
    }

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
