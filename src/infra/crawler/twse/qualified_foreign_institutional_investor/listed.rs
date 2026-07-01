use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use serde_derive::{Deserialize, Serialize};

use crate::{
    core::util::{convert::FromValue, http},
    infra::crawler::share::QfiiDto,
    infra::crawler::twse,
};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
/// TWSE 外資及陸資持股統計 API 回應。
pub struct QFIIResponse {
    /// 回應狀態字串。
    pub stat: Option<String>,
    /// 查詢日期。
    pub date: Option<String>,
    #[serde(rename = "selectType")]
    /// 查詢條件類型。
    pub select_type: String,
    /// 回應標題。
    pub title: Option<String>,
    /// 額外提示文字。
    pub hints: Option<String>,
    /// 欄位名稱清單。
    pub fields: Vec<String>,
    /// 原始資料列。
    pub data: Vec<Vec<serde_json::Value>>,
    /// 總筆數。
    pub total: i32,
}

/// 取得上市股票外資及陸資投資持股統計
pub async fn visit(date_time: DateTime<FixedOffset>) -> Result<Vec<QfiiDto>> {
    let url = format!(
        "https://www.{}/rwd/zh/fund/MI_QFIIS?date={}&selectType=ALLBUT0999&response=json&_={}",
        twse::HOST,
        date_time.format("%Y%m%d"),
        date_time.timestamp_millis()
    );

    let listed = http::get_json::<QFIIResponse>(&url).await?;
    let mut result = Vec::with_capacity(1024);
    let stat = match listed.stat {
        None => {
            tracing::warn!(
                "{}",
                "取得外資及陸資投資持股統計 Finish taiex.Stat is None".to_string(),
            );
            return Ok(result);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        tracing::warn!(
            "{}",
            "取得外資及陸資投資持股統計 Finish taiex.Stat is not ok".to_string(),
        );
        return Ok(result);
    }

    for item in listed.data {
        if item.len() != 12 {
            continue;
        }
        let stock_symbol = item[0].get_string(None);
        let issued_share = item[3].get_i64(None);
        let shares_held = item[5].get_i64(None);
        let share_holding_percentage = item[7].get_decimal(None);

        result.push(QfiiDto {
            stock_symbol,
            issued_share,
            shares_held,
            share_holding_percentage,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::infra::cache::SHARE;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");
        //let date =  DateTime::parse_from_str("2023-09-14 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let datetime_local: DateTime<FixedOffset> =
            match DateTime::parse_from_str("2023-09-15 12:00:00 +0800", "%Y-%m-%d %H:%M:%S %z") {
                Ok(dt) => dt,
                Err(why) => {
                    tracing::debug!("error:{:#?}", why);
                    return;
                }
            };
        match visit(datetime_local).await {
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
            Ok(qfiis) => {
                tracing::debug!("qfiis:{:#?}", qfiis);
            }
        }
        tracing::debug!("結束 visit");
    }
}
