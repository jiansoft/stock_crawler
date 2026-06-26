use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::{core::util, infra::crawler::twse, interfaces::bot};

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
    let stat = match res.stat {
        None => {
            report_error("HolidaySchedule\\.res\\.Stat is None").await;
            return Ok(vec![]);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        report_error("HolidaySchedule\\.res\\.Stat is not ok").await;
        return Ok(vec![]);
    }

    Ok(parse_holiday_data(&res.data))
}

/// 將 API 回應的原始資料列轉換為 `HolidaySchedule`。
///
/// 過濾掉開始交易日（非休市），以及無法解析日期的資料列。
fn parse_holiday_data(data: &[Vec<String>]) -> Vec<HolidaySchedule> {
    data.iter()
        .filter(|d| d.len() >= 3 && !d[2].contains("開始交易"))
        .filter_map(|date_info| {
            NaiveDate::parse_from_str(&date_info[0], "%Y-%m-%d")
                .ok()
                .map(|d| HolidaySchedule {
                    date: d,
                    why: date_info[1].to_string(),
                })
        })
        .collect()
}

async fn report_error(message: &str) {
    bot::telegram::send(message).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cache::SHARE;
    use chrono::Datelike;

    #[test]
    fn test_parse_holiday_data_filters_opening_day() {
        let data = vec![
            vec![
                "2026-01-01".to_string(),
                "中華民國開國紀念日".to_string(),
                "".to_string(),
            ],
            vec![
                "2026-02-27".to_string(),
                "和平紀念日補假".to_string(),
                "".to_string(),
            ],
            // 含「開始交易」的列應被濾除
            vec![
                "2026-01-22".to_string(),
                "龍年開市".to_string(),
                "開始交易".to_string(),
            ],
        ];
        let result = parse_holiday_data(&data);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].date, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(result[0].why, "中華民國開國紀念日");
        assert_eq!(result[1].date, NaiveDate::from_ymd_opt(2026, 2, 27).unwrap());
    }

    #[test]
    fn test_parse_holiday_data_skips_invalid_date() {
        let data = vec![
            vec!["not-a-date".to_string(), "原因".to_string(), "".to_string()],
            vec![
                "2026-12-25".to_string(),
                "聖誕節".to_string(),
                "".to_string(),
            ],
        ];
        let result = parse_holiday_data(&data);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, NaiveDate::from_ymd_opt(2026, 12, 25).unwrap());
    }

    #[test]
    fn test_parse_holiday_data_empty() {
        let result = parse_holiday_data(&[]);
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");
        let now = Local::now();
        match visit(now.date_naive().year()).await {
            Ok(list) => {
                dbg!(&list);
                tracing::debug!("list:{:#?}", list);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
    }
}
