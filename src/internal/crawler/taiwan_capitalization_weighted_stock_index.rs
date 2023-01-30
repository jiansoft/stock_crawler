use crate::internal::request_get;
use crate::logging;
use chrono::Local;
use concat_string::concat_string;
use serde_derive::{Deserialize, Serialize};
use std::{num::ParseFloatError, str::FromStr};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct TaiwanExchangeIndexResponse {
    pub stat: String,
    pub date: String,
    pub title: String,
    pub fields: Vec<String>,
    pub data: Vec<Vec<String>>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
struct Index<'a> {
    pub category: &'a str,
    pub date: &'a str,
    pub index: f64,
    /// 漲跌點數
    pub change: f64,
    /// 成交金額
    pub trade_value: f64,
    /// 成交筆數
    pub transaction: f64,
    /// 成交股數
    pub trading_volume: f64,
}

impl<'a> Index<'a> {
    pub fn new() -> Self {
        Index {
            category: "",
            date: "",
            index: 0.0,
            change: 0.0,
            trade_value: 0.0,
            transaction: 0.0,
            trading_volume: 0.0,
        }
    }

    pub fn fill(&mut self, item: Vec<String>) -> Result<bool, ParseFloatError> {
        self.trading_volume = item[1].replace(",", "").parse::<f64>()?;

        self.trade_value = item[2].replace(",", "").parse::<f64>()?;

        self.transaction = item[3].replace(",", "").parse::<f64>()?;

        self.index = f64::from_str(&*item[4].replace(",", ""))?;

        self.change = f64::from_str(&*item[5].replace(",", ""))?;

        Ok(true)
    }
}

/// 抓台股加權指數
pub async fn visit() {
    let url = concat_string!(
        "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
        Local::now().format("%Y%m%d").to_string(),
        "&_=",
        Local::now().timestamp_millis().to_string()
    );

    if let Some(t) = request_get(url).await {
        //轉成 台股加權 物件
        let taiex = match serde_json::from_str::<TaiwanExchangeIndexResponse>(t.as_str()) {
            Ok(obj) => obj,
            Err(why) => {
                logging::error_file_async(format!(
                    "I can't deserialize an instance of type T from a string of JSON text. because {:?}",
                    why
                ));
                return;
            }
        };

        logging::info_file_async(format!("taiex:{:?}", taiex));

        if taiex.stat.to_uppercase() != "OK" {
            logging::error_file_async("抓取加權股價指數 Finish taiex.Stat is not ok".to_string());
            return;
        }

        for item in taiex.data {
            if item.len() != 6 {
                logging::error_file_async("資料欄位不等於6".to_string());
                continue;
            }

            let split_date: Vec<&str> = item[0].split("/").collect();
            if split_date.len() != 3 {
                logging::error_file_async("日期欄位不等於3".to_string());
                continue;
            }

            let year = match split_date[0].parse::<i64>() {
                Ok(_year) => _year,
                Err(why) => {
                    logging::error_file_async(format!("轉換資料日期發生錯誤. because {:?}", why));
                    continue;
                }
            };

            let mut index = Index::new();
            index.category = "TAIEX";
            let date = format!("{}-{}-{}", year + 1911, split_date[1], split_date[2]);
            index.date = date.as_str();
            let result = index.fill(item);

            match result {
                Ok(_) => {
                    logging::info_file_async(format!("{:?}", index));
                }
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio_test;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn test_visit() {
        aw!(visit());
    }
}
