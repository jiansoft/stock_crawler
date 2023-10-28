use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use serde_derive::{Deserialize, Serialize};

use crate::{
    internal::{
        crawler::twse,
        database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor,
        logging,
    },
    util::http
};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QFIIResponse {
    pub stat: Option<String>,
    pub date: Option<String>,
    #[serde(rename = "selectType")]
    pub select_type: String,
    pub title: Option<String>,
    pub hints: Option<String>,
    pub fields: Vec<String>,
    pub data: Vec<Vec<serde_json::Value>>,
    pub total: i32,
}

/// 取得上市股票外資及陸資投資持股統計
pub async fn visit(
    date_time: DateTime<FixedOffset>,
) -> Result<Vec<QualifiedForeignInstitutionalInvestor>> {
    let url = format!(
        "https://www.{}/rwd/zh/fund/MI_QFIIS?date={}&selectType=ALLBUT0999&response=json&_={}",
        twse::HOST,
        date_time.format("%Y%m%d"),
        date_time.timestamp_millis()
    );

    let listed = http::get_use_json::<QFIIResponse>(&url).await?;
    let mut result = Vec::with_capacity(1024);
    let stat = match listed.stat {
        None => {
            logging::warn_file_async(
                "取得外資及陸資投資持股統計 Finish taiex.Stat is None".to_string(),
            );
            return Ok(result);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        logging::warn_file_async(
            "取得外資及陸資投資持股統計 Finish taiex.Stat is not ok".to_string(),
        );
        return Ok(result);
    }

    for item in listed.data {
        if item.len() != 12 {
            continue;
        }
        let qfii = QualifiedForeignInstitutionalInvestor::from(item);
        result.push(qfii);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::internal::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());
        //let date =  DateTime::parse_from_str("2023-09-14 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let datetime_local: DateTime<FixedOffset> =
            match DateTime::parse_from_str("2023-09-15 12:00:00 +0800", "%Y-%m-%d %H:%M:%S %z") {
                Ok(dt) => dt,
                Err(why) => {
                    logging::debug_file_async(format!("error:{:#?}", why));
                    return;
                }
            };
        match visit(datetime_local).await {
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
            Ok(qfiis) => {
                logging::debug_file_async(format!("qfiis:{:#?}", qfiis));
            }
        }
        logging::debug_file_async("結束 visit".to_string());
    }
}
