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
/// TWSE `MI_INDEX` API 的回應主體。
///
/// 這個 crawler 只使用 `tables` 中的「上市每日收盤行情」表格資料。
pub struct ListedResponse {
    pub stat: Option<String>,
    #[serde(rename = "tables")]
    pub tables: Vec<Table>,
}

#[derive(Serialize, Deserialize, Debug)]
/// TWSE `MI_INDEX` 回應中的單一表格區塊。
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

/// 判斷是否為 TWSE 上市每日收盤行情表格。
///
/// 因 TWSE `tables` 順序可能調整，這裡改用欄位名稱特徵比對，
/// 避免依賴固定索引位置。
fn is_twse_quote_table(table: &Table) -> bool {
    let Some(fields) = &table.fields else {
        return false;
    };

    if fields.len() < 16 {
        return false;
    }

    let required = ["證券代號", "成交股數", "開盤價", "收盤價", "漲跌價差"];
    required.iter().all(|key| fields.iter().any(|f| f.contains(key)))
}

/// 從 `MI_INDEX` 回應中找出上市每日收盤行情的資料列。
///
/// 回傳值為對應表格的 `data` 欄位引用，若找不到則回傳 `None`。
fn find_twse_quote_rows(data: &ListedResponse) -> Option<&Vec<Vec<String>>> {
    data.tables
        .iter()
        .find(|table| is_twse_quote_table(table))
        .and_then(|table| table.data.as_ref())
}

/// 抓取上市公司每日收盤資訊
///
/// 資料來源：`/exchangeReport/MI_INDEX?type=ALLBUT0999`
///
/// 函式會：
/// 1. 下載 TWSE JSON 並動態定位目標表格。
/// 2. 轉換為 `DailyQuote`，過濾無效價格與記憶體去重資料。
/// 3. 盡可能計算 `change_range` 後回傳當日清單。
///
/// 若 API 可連線但找不到目標表格，會記錄 warning 並回傳空陣列。
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
    if let Some(twse_dqs) = find_twse_quote_rows(&data) {
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
                        dq.change_range =
                            (dq.closing_price - ldg.closing_price) / ldg.closing_price * dec!(100);
                    } else if dq.opening_price > Decimal::ZERO {
                        dq.change_range = dq.change / dq.opening_price * dec!(100);
                    } else {
                        dq.change_range = Decimal::ZERO;
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
    } else {
        logging::warn_file_async(format!(
            "TWSE MI_INDEX quote table not found for date={}, stat={:?}, tables={}",
            date,
            data.stat,
            data.tables.len()
        ));
    }
    Ok(dqs)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Timelike};
    use std::time::Duration;

    use crate::{cache::SHARE, logging};

    use super::*;

    #[test]
    fn test_is_twse_quote_table() {
        let table = Table {
            title: Some("每日收盤行情(全部(不含權證、牛熊證))".to_string()),
            fields: Some(vec![
                "證券代號".to_string(),
                "證券名稱".to_string(),
                "成交股數".to_string(),
                "成交筆數".to_string(),
                "成交金額".to_string(),
                "開盤價".to_string(),
                "最高價".to_string(),
                "最低價".to_string(),
                "收盤價".to_string(),
                "漲跌(+/-)".to_string(),
                "漲跌價差".to_string(),
                "最後揭示買價".to_string(),
                "最後揭示買量".to_string(),
                "最後揭示賣價".to_string(),
                "最後揭示賣量".to_string(),
                "本益比".to_string(),
            ]),
            data: None,
            hints: None,
        };

        assert!(is_twse_quote_table(&table));
    }

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
