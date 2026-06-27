use anyhow::Result;
use chrono::{DateTime, Local};
use serde_derive::{Deserialize, Serialize};

use crate::{core::util, infra::crawler::twse};

/// 調用台股指數 twse API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct TaiwanStockIndexDto {
    /// API 回應狀態。
    pub stat: String,
    /// 查詢日期。
    pub date: Option<String>,
    /// 回應標題。
    pub title: Option<String>,
    /// 欄位名稱清單。
    pub fields: Option<Vec<String>>,
    /// 原始指數資料列。
    pub data: Option<Vec<Vec<String>>>,
}

/// 取得台股指數
pub async fn visit(date: DateTime<Local>) -> Result<TaiwanStockIndexDto> {
    let url = format!(
        "https://www.{}/exchangeReport/FMTQIK?response=json&date={}&_={}",
        twse::HOST,
        date.format("%Y%m%d"),
        date.timestamp_millis()
    );

    util::http::get_json::<TaiwanStockIndexDto>(&url).await
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");

        match visit(Local::now()).await {
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
            Ok(list) => {
                tracing::debug!("data:{:#?}", list);
            }
        }
        tracing::debug!("結束 visit");
    }
}
