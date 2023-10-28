use anyhow::*;
use chrono::{DateTime, Local};
use serde_derive::{Deserialize, Serialize};

use crate::{internal::crawler::twse, util};

/// 調用台股指數 twse API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct Index {
    pub stat: String,
    pub date: Option<String>,
    pub title: Option<String>,
    pub fields: Option<Vec<String>>,
    pub data: Option<Vec<Vec<String>>>,
}

/// 取得台股指數
pub async fn visit(date: DateTime<Local>) -> Result<Index> {
    let url = format!(
        "https://www.{}/exchangeReport/FMTQIK?response=json&date={}&_={}",
        twse::HOST,
        date.format("%Y%m%d"),
        date.timestamp_millis()
    );

    util::http::get_use_json::<Index>(&url).await
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit(Local::now()).await {
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
            Ok(list) => {
                logging::debug_file_async(format!("data:{:#?}", list));
            }
        }
        logging::debug_file_async("結束 visit".to_string());
    }
}
