use crate::{internal::util, logging};
use chrono::{DateTime, Local};
use concat_string::concat_string;
use serde_derive::{Deserialize, Serialize};

/// 調用台股指數 twse API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub stat: String,
    pub date: Option<String>,
    pub title: Option<String>,
    pub fields: Option<Vec<String>>,
    pub data: Option<Vec<Vec<String>>>,
}

/// 取得台股指數
pub async fn visit(date: DateTime<Local>) -> Option<Entity> {
    let url = concat_string!(
        "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
        date.format("%Y%m%d").to_string(),
        "&_=",
        date.timestamp_millis().to_string()
    );

    logging::info_file_async(format!("visit url:{}", url,));

    util::http::request_get_use_json::<Entity>(&url)
        .await
        .map_err(|why| {
            logging::error_file_async(format!("Failed to request_get_use_json because {:?}", why));
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use crate::internal::cache_share::CACHE_SHARE;
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit(Local::now()).await {
            None => {
                logging::debug_file_async(
                    "Failed to visit because response is no data".to_string(),
                );
            }
            Some(result) => {
                logging::debug_file_async(format!("result:{:#?}", result));
            }
        }
        logging::debug_file_async("結束 visit".to_string());
    }
}
