use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{bot, crawler::twse, logging, util, util::map::Keyable};

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
struct PublicFormResponse {
    pub stat: Option<String>,
    pub date: String,
    pub title: String,
    pub fields: Vec<String>,
    pub data: Vec<Vec<String>>,
    pub notes: Vec<String>,
    pub total: i64,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
pub struct Public {
    pub stock_symbol: String,
    pub stock_name: String,
    /// 發行市場
    pub market: String,
    /// 申購開始日
    pub offering_start_date: Option<NaiveDate>,
    /// 申購結束日
    pub offering_end_date: Option<NaiveDate>,
    /// 抽籤日期
    pub drawing_date: Option<NaiveDate>,
    /// 承銷價
    pub offering_price: Option<Decimal>,
    /// 撥券日期
    pub issue_date: Option<NaiveDate>,
}

impl Keyable for Public {
    fn key(&self) -> String {
        self.stock_symbol.clone()
    }

    fn key_with_prefix(&self) -> String {
        format!("Public:{}", self.key())
    }
}

impl Public {
    pub fn new(stock_symbol: String, stock_name: String, market: String) -> Self {
        Self {
            stock_symbol,
            stock_name,
            market,
            offering_start_date: Default::default(),
            offering_end_date: Default::default(),
            drawing_date: Default::default(),
            offering_price: Default::default(),
            issue_date: Default::default(),
        }
    }
}

pub async fn visit() -> Result<Vec<Public>> {
    let now = Local::now();
    let date = now + Duration::days(5);
    let url = format!(
            "https://www.{host}/rwd/zh/announcement/publicForm?date={year}&response=json&_={time}",
        host = twse::HOST,
        year = date.year(),
        time = now.timestamp_millis()
    );
    let res = util::http::get_use_json::<PublicFormResponse>(&url).await?;
    let mut result: Vec<Public> = Vec::with_capacity(2048);
    let stat = match res.stat {
        None => {
            let to_bot_msg = "Public.res.Stat is None";
            if let Err(why) = bot::telegram::send(to_bot_msg).await {
                logging::error_file_async(format!("Failed to send because {:?}", why));
            }
            return Ok(result);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        let to_bot_msg = "Public.res.Stat is not ok";
        if let Err(why) = bot::telegram::send(to_bot_msg).await {
            logging::error_file_async(format!("Failed to send because {:?}", why));
        }
        return Ok(result);
    }

    for item in res.data {
        // ["序號", "抽籤日期", "證券名稱", "證券代號", "發行市場",
        //  5"申購開始日", 6"申購結束日", "承銷股數", "實際承銷股數", "承銷價(元)",
        // 10 "實際承銷價(元)", 撥券日期(上市、上櫃日期)]
        let mut p = Public::new(item[3].clone(), item[2].clone(), item[4].clone());
        p.drawing_date = util::datetime::parse_taiwan_date(&item[1]);
        p.offering_start_date = util::datetime::parse_taiwan_date(&item[5]);
        p.offering_end_date = util::datetime::parse_taiwan_date(&item[6]);
        p.issue_date = util::datetime::parse_taiwan_date(&item[11]);
        p.offering_price = util::text::parse_decimal(&item[10], None).ok();

        result.push(p);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit().await {
            Ok(list) => {
                dbg!(&list);
                logging::debug_file_async(format!("list:{:#?}", list));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
