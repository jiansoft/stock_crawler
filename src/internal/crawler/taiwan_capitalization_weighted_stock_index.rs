use crate::{
    internal::database::model,
    internal::{cache_share, database, request_get},
    logging,
};
use chrono::{Local, NaiveDate};
use concat_string::concat_string;
use serde_derive::{Deserialize, Serialize};
use sqlx::{postgres::PgQueryResult, Error};
use std::str::FromStr;

/// 調用台股指數 twse API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct TaiwanExchangeIndexResponse {
    pub stat: String,
    pub date: String,
    pub title: String,
    pub fields: Vec<String>,
    pub data: Vec<Vec<String>>,
}

/// 台股指數存於資籿庫內的數據
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

    /// date與 category 為組合鍵 unique
    pub async fn upsert(&self) -> Result<PgQueryResult, Error> {
        let sql = r#"
insert into index (
    category, "date", trading_volume, "transaction", trade_value, change, index, create_time, update_time
) values (
    $1,$2,$3,$4,$5,$6,$7,$8,$9
) ON CONFLICT ("date",category) DO UPDATE SET update_time = excluded.update_time;
        "#;
        sqlx::query(sql)
            .bind(self.category)
            .bind(self.date)
            .bind(self.trading_volume)
            .bind(self.transaction)
            .bind(self.trade_value)
            .bind(self.change)
            .bind(self.index)
            .bind(self.create_time)
            .bind(self.update_time)
            .execute(&database::DB.pool)
            .await
    }
}

/// 調用  twse API 取得台股加權指數
pub async fn visit() {
    let url = concat_string!(
        "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
        Local::now().format("%Y%m%d").to_string(),
        "&_=",
        Local::now().timestamp_millis().to_string()
    );

    logging::info_file_async(format!("visit url:{}", url,));

    match request_get(url).await {
        Ok(t) => {
            //轉成 台股加權 物件
            let taiex = match serde_json::from_slice::<TaiwanExchangeIndexResponse>(t.as_bytes()) {
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
                logging::error_file_async(
                    "抓取加權股價指數 Finish taiex.Stat is not ok".to_string(),
                );
                return;
            }

            for item in taiex.data {
                if item.len() != 6 {
                    logging::error_file_async("資料欄位不等於6".to_string());
                    continue;
                }

                let split_date: Vec<&str> = item[0].split('/').collect();
                if split_date.len() != 3 {
                    logging::error_file_async("日期欄位不等於3".to_string());
                    continue;
                }

                let year = match split_date[0].parse::<i64>() {
                    Ok(_year) => _year,
                    Err(why) => {
                        logging::error_file_async(format!(
                            "轉換資料日期發生錯誤. because {:?}",
                            why
                        ));
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
                // logging::info_file_async(format!("visit_key:{}", key));
                if cache_share::CACHE_SHARE
                    .indices
                    .read()
                    .unwrap()
                    .contains_key(key.as_str())
                {
                    //logging::info_file_async(format!("指數已存在 {:?}", key));
                    continue;
                }

                index.trading_volume = match item[1].replace(',', "").parse::<f64>() {
                    Ok(_trading_volume) => _trading_volume,
                    Err(_) => continue,
                };

                index.trade_value = match item[2].replace(',', "").parse::<f64>() {
                    Ok(_trade_value) => _trade_value,
                    Err(_) => continue,
                };

                index.transaction = match item[3].replace(',', "").parse::<f64>() {
                    Ok(_transaction) => _transaction,
                    Err(_) => continue,
                };

                index.index = match f64::from_str(&item[4].replace(',', "")) {
                    Ok(_index) => _index,
                    Err(_) => continue,
                };

                index.change = match f64::from_str(&item[5].replace(',', "")) {
                    Ok(_change) => _change,
                    Err(_) => continue,
                };

                match index.upsert().await {
                    Ok(_) => {
                        logging::info_file_async(format!("index add {:?}", index));
                        match cache_share::CACHE_SHARE.indices.write() {
                            Ok(mut indices) => {
                                indices
                                    .insert(key, model::index::Entity::from_index_response(&index));
                            }
                            Err(why) => {
                                logging::error_file_async(format!("Failed to write stocks cache because {:?}", why));
                            }
                        }
                    }
                    Err(why) => {
                        logging::error_file_async(format!("Failed to upsert because {:?}", why));
                    }
                }
            }
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to request_get because {:?}", why));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::internal::cache_share::CACHE_SHARE;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        visit().await;
    }

    #[tokio::test]
    async fn test_index_upsert() {
        dotenv::dotenv().ok();
        let mut index = Index::new();
        index.category = "TAIEX";
        index.date = NaiveDate::from_ymd_opt(2023, 1, 31).unwrap();

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
