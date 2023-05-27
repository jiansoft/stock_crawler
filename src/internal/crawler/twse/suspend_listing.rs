use crate::internal::{logging, util};
use anyhow::*;
use serde::Deserialize;

/// 調用 twse suspendListingCsvAndHtml API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
pub struct SuspendListing {
    #[serde(rename(deserialize = "DelistingDate"))]
    pub delisting_date: String,
    #[serde(rename(deserialize = "Company"))]
    pub name: String,
    #[serde(rename(deserialize = "Code"))]
    pub stock_symbol: String,
}

/// 取得終止上市公司名單
pub async fn visit() -> Result<Vec<SuspendListing>> {
    let url = "https://openapi.twse.com.tw/v1/company/suspendListingCsvAndHtml";
    logging::info_file_async(format!("visit url:{}", url));
    util::http::get_use_json::<Vec<SuspendListing>>(url).await
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;
    use std::result::Result::Ok;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit().await {
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
