use crate::internal::database::model;
use crate::internal::{cache_share, database, request_get};
use crate::logging;
use chrono::{Local, NaiveDate};
use concat_string::concat_string;
use serde_derive::{Deserialize, Serialize};
use sqlx::postgres::PgQueryResult;
use sqlx::Error;
use std::str::FromStr;

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
pub struct Index<'a> {
    pub category: &'a str,
    pub date: NaiveDate,
    pub index: f64,
    /// 漲跌點數
    pub change: f64,
    /// 成交金額
    pub trade_value: f64,
    /// 成交筆數
    pub transaction: f64,
    /// 成交股數
    pub trading_volume: f64,
    pub create_time: chrono::DateTime<Local>,
    pub update_time: chrono::DateTime<Local>,
}

impl<'a> Index<'a> {
    pub fn new() -> Self {
        Index {
            category: "",
            date: Default::default(),
            index: 0.0,
            change: 0.0,
            trade_value: 0.0,
            transaction: 0.0,
            trading_volume: 0.0,
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }

    pub async fn upsert(&self) -> Result<PgQueryResult, Error> {
        let sql = r#"
insert into index (
    category, "date", trading_volume, "transaction", trade_value, change, index, update_time
) values (
    $1,$2,$3,$4,$5,$6,$7,$8
) ON CONFLICT ("date",category) DO UPDATE SET update_time = excluded.update_time;;
        "#;
        sqlx::query(sql)
            .bind(self.category)
            .bind(self.date)
            .bind(self.trading_volume)
            .bind(self.transaction)
            .bind(self.trade_value)
            .bind(self.change)
            .bind(self.index)
            .bind(self.update_time)
            .execute(&database::DB.db)
            .await
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
            let date = concat_string!(
                (year + 1911).to_string(),
                "-",
                split_date[1],
                "-",
                split_date[2]
            );

            index.date = NaiveDate::from_str(date.as_str()).unwrap();
            let key = index.date.to_string() + "_" + index.category;
            logging::info_file_async(format!("visit_key:{}", key));
            if cache_share::CACHE_SHARE
                .indices
                .read()
                .unwrap()
                .contains_key(key.as_str())
            {
                logging::info_file_async(format!("指數已存在 {:?}", key));
                continue;
            }

            index.trading_volume = match item[1].replace(",", "").parse::<f64>() {
                Ok(_trading_volume) => _trading_volume,
                Err(_) => continue,
            };

            index.trade_value = match item[2].replace(",", "").parse::<f64>() {
                Ok(_trade_value) => _trade_value,
                Err(_) => continue,
            };

            index.transaction = match item[3].replace(",", "").parse::<f64>() {
                Ok(_transaction) => _transaction,
                Err(_) => continue,
            };

            index.index = match f64::from_str(&*item[4].replace(",", "")) {
                Ok(_index) => _index,
                Err(_) => continue,
            };

            index.change = match f64::from_str(&*item[5].replace(",", "")) {
                Ok(_change) => _change,
                Err(_) => continue,
            };

            match index.upsert().await {
                Ok(_) => {
                    cache_share::CACHE_SHARE
                        .indices
                        .write()
                        .unwrap()
                        .insert(key, model::index::Entity::from_index_response(index.clone()));
                    logging::info_file_async(format!("{:?}", index));
                }
                Err(why) => {
                    logging::error_file_async(format!("because {:?}", why));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        //cache_share::CACHE_SHARE.init().await;
        visit().await;
    }

    #[tokio::test]
    async fn test_index_upsert() {
        dotenv::dotenv().ok();
        let mut index = Index::new();
        index.category = "TAIEX";
        index.date = NaiveDate::from_ymd_opt(2023, 01, 31).unwrap();

        match index.upsert().await {
            Ok(_) => {
                logging::info_file_async("結束".to_string());
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        };
    }
    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn test_update() {
        dotenv::dotenv().ok();
        aw!(visit());
    }
}
