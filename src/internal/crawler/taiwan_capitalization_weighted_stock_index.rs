use crate::{internal::cache_share, internal::database::model, internal::util, logging};
use chrono::{Local, NaiveDate};
use concat_string::concat_string;
use rust_decimal::Decimal;
use serde_derive::{Deserialize, Serialize};
use std::str::FromStr;

/// 調用台股指數 twse API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct TaiwanExchangeIndexResponse {
    pub stat: String,
    pub date: Option<String>,
    pub title: Option<String>,
    pub fields: Option<Vec<String>>,
    pub data: Option<Vec<Vec<String>>>,
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

    match util::http::request_get_use_json::<TaiwanExchangeIndexResponse>(&url).await {
        Ok(tai_ex) => {
            logging::info_file_async(format!("tai_ex:{:?}", tai_ex));
            if tai_ex.stat.to_uppercase() != "OK" {
                logging::info_file_async(
                    "抓取加權股價指數 Finish taiex.Stat is not ok".to_string(),
                );
                return;
            }

            if let Some(data) = tai_ex.data {
                for item in data {
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

                    let mut index = model::index::Entity::new();
                    index.category = String::from("TAIEX");
                    let date = concat_string!(
                        (year + 1911).to_string(),
                        "-",
                        split_date[1],
                        "-",
                        split_date[2]
                    );

                    index.date = NaiveDate::from_str(date.as_str()).unwrap();
                    let key = index.date.to_string() + "_" + &index.category;
                    if let Ok(indices) = cache_share::CACHE_SHARE.indices.read() {
                        if indices.contains_key(key.as_str()) {
                            continue;
                        }
                    }

                    index.trading_volume = match Decimal::from_str(&item[1].replace(',', "")) {
                        Ok(_trading_volume) => _trading_volume,
                        Err(_) => continue,
                    };

                    index.trade_value = match Decimal::from_str(&item[2].replace(',', "")) {
                        Ok(_trade_value) => _trade_value,
                        Err(_) => continue,
                    };

                    index.transaction = match Decimal::from_str(&item[3].replace(',', "")) {
                        Ok(_transaction) => _transaction,
                        Err(_) => continue,
                    };

                    index.index = match Decimal::from_str(&item[4].replace(',', "")) {
                        Ok(_index) => _index,
                        Err(_) => continue,
                    };

                    index.change = match Decimal::from_str(&item[5].replace(',', "")) {
                        Ok(_change) => _change,
                        Err(_) => continue,
                    };

                    match index.upsert().await {
                        Ok(_) => {
                            logging::info_file_async(format!("index add {:?}", index));
                            match cache_share::CACHE_SHARE.indices.write() {
                                Ok(mut indices) => {
                                    indices.insert(key, index);
                                }
                                Err(why) => {
                                    logging::error_file_async(format!(
                                        "Failed to write stocks cache because {:?}",
                                        why
                                    ));
                                }
                            }
                        }
                        Err(why) => {
                            logging::error_file_async(format!(
                                "Failed to upsert because {:?}",
                                why
                            ));
                        }
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
    use crate::internal::util;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        visit().await;
    }

    #[tokio::test]
    async fn test_visit_http() {
        dotenv::dotenv().ok();
        let url = concat_string!(
            "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
            Local::now().format("%Y%m%d").to_string(),
            "&_=",
            Local::now().timestamp_millis().to_string()
        );
        match util::http::request_get_use_json::<TaiwanExchangeIndexResponse>(&url).await {
            Ok(tai) => {
                logging::info_file_async(format!("tai{:#?}", tai));
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }
    }

    /*#[tokio::test]
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
    }*/

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
