use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

// 通知改走 core::alert 抽象介面，infra 層不直接依賴 interfaces::bot（反向耦合）。
use crate::{core::alert, core::util, infra::crawler::twse, infra::nosql::redis::CLIENT};

/// 最近一次成功取得的休市日表在 Redis 的保存時間（秒）：400 天，跨年查前一年也還在。
const BACKUP_TTL_SECONDS: usize = 400 * 24 * 60 * 60;

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

/// 取得指定年度的休市日清單；證交所讀不到或回空清單時，改用 Redis 裡最近一次成功的結果。
///
/// 11-07 健康週報（週六 11:00）證交所連續逾時三次，缺漏交易日檢查因此整段略過；休市日表一年只公布
/// 一次、很少變動，用前一次的結果比略過檢查可靠。成功取得時順手更新備份（寫入失敗只記 DEBUG）。
/// 需要「讀不到就停下」的呼叫端（盤中追蹤判斷今天是否開市）仍用 [`visit`]。
///
/// # 錯誤
///
/// 證交所讀不到且 Redis 沒有該年度的備份時，回傳證交所的錯誤。
pub async fn visit_or_backup(year: i32) -> Result<Vec<HolidaySchedule>> {
    let fetched = visit(year).await;
    if let Ok(schedule) = &fetched
        && !schedule.is_empty()
    {
        save_backup(year, schedule).await;
        return fetched;
    }
    match load_backup(year).await {
        Some(schedule) => {
            let days = schedule.len();
            tracing::warn!("{year} 年休市日表讀不到或是空的，改用 Redis 備份（{days} 天）");
            Ok(schedule)
        }
        None => fetched,
    }
}

/// 休市日表備份的 Redis key。
fn backup_key(year: i32) -> String {
    format!("twse:holiday_schedule:{year}")
}

/// 把成功取得的休市日表寫進 Redis；失敗只記 DEBUG，不影響本次結果。
async fn save_backup(year: i32, schedule: &[HolidaySchedule]) {
    let Ok(json) = serde_json::to_string(schedule) else {
        return;
    };
    if let Err(why) = CLIENT.set(backup_key(year), json, BACKUP_TTL_SECONDS).await {
        tracing::debug!("寫入 {year} 年休市日表備份失敗: {why:?}");
    }
}

/// 讀 Redis 裡該年度的休市日表備份；沒有、讀不到或格式不符時為 `None`。
async fn load_backup(year: i32) -> Option<Vec<HolidaySchedule>> {
    let json = CLIENT.get_string(&backup_key(year)).await.ok()?;
    serde_json::from_str(&json).ok()
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
    alert::send_message(message).await;
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
        assert_eq!(
            result[1].date,
            NaiveDate::from_ymd_opt(2026, 2, 27).unwrap()
        );
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
        assert_eq!(
            result[0].date,
            NaiveDate::from_ymd_opt(2026, 12, 25).unwrap()
        );
    }

    #[test]
    fn test_parse_holiday_data_empty() {
        let result = parse_holiday_data(&[]);
        assert!(result.is_empty());
    }

    /// 以本機 mock server 取代證交所休市日表。
    mod backup {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::core::util::http::mock_origin::with_origins;

        const TWSE: &str = "https://www.twse.com.tw";

        async fn server(year: &str, body: ResponseTemplate) -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/rwd/zh/holidaySchedule/holidaySchedule"))
                .and(query_param("date", year))
                .respond_with(body)
                .mount(&server)
                .await;
            server
        }

        fn holiday(year: i32) -> HolidaySchedule {
            HolidaySchedule {
                date: NaiveDate::from_ymd_opt(year, 10, 10).expect("有效日期"),
                why: "國慶日".to_string(),
            }
        }

        fn ok_body(year: i32) -> ResponseTemplate {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "ok", "date": format!("{year}0101"), "queryYear": year, "total": 1,
                "data": [[format!("{year}-10-10"), "國慶日", "依規定放假1日。"]]
            }))
        }

        fn broken_body() -> ResponseTemplate {
            ResponseTemplate::new(200).set_body_string("not json")
        }

        /// 證交所讀不到、Redis 也沒有備份時回傳證交所的錯誤（1985 年從不寫備份）。
        #[tokio::test]
        async fn visit_or_backup_fails_without_backup() {
            let server = server("1985", broken_body()).await;
            let uri = server.uri();
            let result = with_origins(&[(TWSE, uri.as_str())], visit_or_backup(1985)).await;
            assert!(result.is_err(), "沒有備份時不能當成沒有休市日");
        }

        /// 連 Redis：成功時寫備份；之後證交所讀不到就讀備份。
        #[tokio::test]
        #[cfg_attr(
            not(feature = "integration-tests"),
            ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
        )]
        async fn visit_or_backup_falls_back_to_last_success() {
            dotenvy::dotenv().ok();
            if CLIENT.ping().await.is_err() {
                println!("跳過 visit_or_backup_falls_back_to_last_success：無 Redis 連接");
                return;
            }
            const YEAR: i32 = 1986;
            CLIENT.delete(&backup_key(YEAR)).await.expect("清掉舊備份");

            let ok = server("1986", ok_body(YEAR)).await;
            let uri = ok.uri();
            let fetched = with_origins(&[(TWSE, uri.as_str())], visit_or_backup(YEAR))
                .await
                .expect("證交所可讀");
            assert_eq!(fetched, vec![holiday(YEAR)]);
            assert_eq!(
                load_backup(YEAR).await,
                Some(vec![holiday(YEAR)]),
                "成功時寫備份"
            );

            let broken = server("1986", broken_body()).await;
            let uri = broken.uri();
            let fallback = with_origins(&[(TWSE, uri.as_str())], visit_or_backup(YEAR))
                .await
                .expect("讀不到時改用備份");
            assert_eq!(fallback, vec![holiday(YEAR)]);

            CLIENT.delete(&backup_key(YEAR)).await.expect("清理備份");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
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
