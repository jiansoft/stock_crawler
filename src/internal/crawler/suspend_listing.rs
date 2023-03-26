use crate::{internal::cache_share::CACHE_SHARE, internal::request_get, logging};
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
    logging::info_file_async(format!("visit url:{}", url));

    match request_get(url).await {
        Ok(t) => {
            let mut items_to_update = Vec::new();
            match serde_json::from_slice::<Vec<SuspendListing>>(t.as_bytes()) {
                Ok(delisting) => match CACHE_SHARE.stocks.read() {
                    Ok(stocks) => {
                        for item in delisting {
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
                                items_to_update.push(another);
                            }
                        }
                    }
                    Err(why) => {
                        logging::error_file_async(format!(
                            "Failed to write stocks cache because {:?}",
                            why
                        ));
                    }
                },
                Err(why) => {
                    logging::error_file_async(format!(
                        "I can't deserialize an instance of type T from a string of JSON text. because {:?}",
                        why
                    ));
                }
            };

            let mut updated_stocks = Vec::with_capacity(items_to_update.len());

            for item in items_to_update {
                match item.update_suspend_listing().await {
                    Ok(_) => {
                        updated_stocks.push(item);
                    }
                    Err(why) => {
                        logging::error_file_async(format!(
                            "Failed to update_suspend_listing because {:?}",
                            why
                        ));
                    }
                }
            }

            /*  if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
                for stock in updated_stocks {
                    stocks.insert(stock.security_code.clone(), stock);
                }
            }*/

            let to_stocks = updated_stocks
                .into_iter()
                .map(|stock| (stock.security_code.clone(), stock));
            if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
                stocks.extend(to_stocks);
            }
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to request_get because {:?}", why));
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
