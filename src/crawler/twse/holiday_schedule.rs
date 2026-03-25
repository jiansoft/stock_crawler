use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::{bot, crawler::twse, util};

#[derive(Serialize, Deserialize)]
struct HolidayScheduleResponse {
    pub stat: Option<String>,
    pub date: String,
    pub data: Vec<Vec<String>>,
    #[serde(rename = "queryYear")]
    pub query_year: i64,
    pub total: i64,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
/// 交易所休市日資料。
pub struct HolidaySchedule {
    /// 休市日期。
    pub date: NaiveDate,
    /// 休市原因。
    pub why: String,
}

/// 取得指定年度的休市日清單。
///
/// # 參數
///
/// * `year` - 西元年
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 解析失敗時回傳錯誤。
pub async fn visit(year: i32) -> Result<Vec<HolidaySchedule>> {
    let now = Local::now();
    let url = format!(
        "https://www.{host}/rwd/zh/holidaySchedule/holidaySchedule?date={year}&response=json&_={time}",
        host = twse::HOST,
        year = year,
        time = now.timestamp_millis()
    );
    let res = util::http::get_json::<HolidayScheduleResponse>(&url).await?;
    let mut result: Vec<HolidaySchedule> = Vec::with_capacity(32);
    let stat = match res.stat {
        None => {
            report_error("HolidaySchedule\\.res\\.Stat is None").await;
            return Ok(result);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        report_error("HolidaySchedule\\.res\\.Stat is not ok").await;
        return Ok(result);
    }

    for date_info in res
        .data
        .iter()
        .filter(|d| d.len() >= 3 && !d[2].contains("開始交易"))
    {
        if let Ok(d) = NaiveDate::parse_from_str(&date_info[0], "%Y-%m-%d") {
            result.push(HolidaySchedule {
                date: d,
                why: date_info[1].to_string(),
            });
        }
    }

    Ok(result)
}

async fn report_error(message: &str) {
    bot::telegram::send(message).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SHARE;
    use crate::logging;
    use chrono::Datelike;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());
        let now = Local::now();
        match visit(now.date_naive().year()).await {
            Ok(list) => {
                dbg!(&list);
                logging::debug_file_async(format!("list:{:#?}", list));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
