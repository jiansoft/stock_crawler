use crate::{
    internal::cache_share::CACHE_SHARE,
    internal::request_get,
    logging
};
use serde::Deserialize;

/// 調用 twse suspendListingCsvAndHtml API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
pub struct SuspendListing {
    #[serde(rename(deserialize = "DelistingDate"))]
    pub delisting_date: String,
    #[serde(rename(deserialize = "Company"))]
    pub name: String,
    #[serde(rename(deserialize = "Code"))]
    pub security_code: String,
}

pub async fn visit() {
    let url = "https://openapi.twse.com.tw/v1/company/suspendListingCsvAndHtml";

    if let Some(t) = request_get(url).await {
        let mut need_to_update_items = Vec::new();
        match serde_json::from_str::<Vec<SuspendListing>>(t.as_str()) {
            Ok(delisting) => {
                for item in delisting {
                    match CACHE_SHARE.stocks.read() {
                        Ok(stocks) => {
                            if let Some(stock) = stocks.get(item.security_code.as_str()) {
                                 if stock.suspend_listing {
                                    continue;
                                }

                                let year = match item.delisting_date[..3].parse::<i32>() {
                                    Ok(_year) => _year,
                                    Err(why) => {
                                        logging::error_file_async(format!(
                                            "轉換資料日期發生錯誤. because {:?}",
                                            why
                                        ));
                                        continue;
                                    }
                                };


                                if year < 110 {
                                    continue;
                                }

                                let mut another = stock.clone();
                                another.suspend_listing = true;
                                need_to_update_items.push(another);
                            }
                        }
                        Err(why) => {
                            logging::error_file_async(format!("because {:?}", why));
                        }
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "I can't read the config context because {:?}",
                    why
                ));
            }
        };

        for item in need_to_update_items {
            match item.update_suspend_listing().await {
                Ok(_) => {
                    if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
                        stocks.insert(item.security_code.to_string(), item);
                    }
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
        CACHE_SHARE.load().await;
        visit().await;
    }
}
