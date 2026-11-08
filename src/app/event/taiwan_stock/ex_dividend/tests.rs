//! 除權息提醒排程的測試；連外的測試標 #[ignore]。

use std::time::Duration;

use tokio::time;

use super::*;

#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 execute");
    //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
    //let today: NaiveDate = Local::today().naive_local();
    let _ = execute().await;

    tracing::info!("結束 execute");
    time::sleep(Duration::from_secs(1)).await;
}

#[test]
fn test_is_trading_day() {
    let mut holidays = HashSet::new();
    // 2026-05-25 (週一)
    let monday = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
    // 2026-05-24 (週日)
    let sunday = NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
    // 2026-05-23 (週六)
    let saturday = NaiveDate::from_ymd_opt(2026, 5, 23).unwrap();

    // 未設定休市日時，週一應為交易日，週末非交易日
    assert!(is_trading_day(monday, &holidays));
    assert!(!is_trading_day(sunday, &holidays));
    assert!(!is_trading_day(saturday, &holidays));

    // 將週一設為休市日
    holidays.insert(monday);
    assert!(!is_trading_day(monday, &holidays));
}

#[test]
fn test_find_next_trading_day() {
    let mut holidays = HashSet::new();
    // 2026-05-22 (週五)
    let friday = NaiveDate::from_ymd_opt(2026, 5, 22).unwrap();
    // 2026-05-25 (週一)
    let monday = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
    // 2026-05-26 (週二)
    let tuesday = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();

    // 正常週五的下一交易日應為週一
    assert_eq!(find_next_trading_day(friday, &holidays), monday);

    // 若週一為節假日休市，則週五的下一交易日應為週二
    holidays.insert(monday);
    assert_eq!(find_next_trading_day(friday, &holidays), tuesday);
}

/// 以本機 mock server 取代證交所休市日 API（G15），驗證載入與降級（G17）。
mod mock_sources {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const HOLIDAY_PATH: &str = "/rwd/zh/holidaySchedule/holidaySchedule";

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2019, month, day).expect("固定測試日期")
    }

    async fn holidays(server: &MockServer) -> HashSet<NaiveDate> {
        let uri = server.uri();
        with_origins(&[("https://www.twse.com.tw", &uri)], get_holidays_set(2019)).await
    }

    /// 只收休市日：「開始交易」列與日期無法解析的列都略過。
    #[tokio::test]
    async fn get_holidays_set_collects_closed_dates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(HOLIDAY_PATH))
            .and(query_param("date", "2019"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "ok",
                "date": "20190101",
                "queryYear": 2019,
                "total": 3,
                "data": [
                    ["2019-01-02", "開始交易", "國曆新年開始交易日。"],
                    ["2019-02-04", "農曆春節", "依規定放假"],
                    ["不是日期", "其他", "無法解析"]
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let set = holidays(&server).await;

        assert_eq!(set, HashSet::from([day(2, 4)]));
        assert!(!is_trading_day(day(2, 4), &set), "春節休市不應算交易日");
        assert_eq!(
            find_next_trading_day(day(2, 1), &set),
            day(2, 5),
            "週五之後跳過週末與 02-04 休市"
        );
    }

    /// 休市日 API 失敗時降級為空集合，只依週末判斷交易日。
    #[tokio::test]
    async fn get_holidays_set_falls_back_to_empty_on_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(HOLIDAY_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let set = holidays(&server).await;

        assert!(set.is_empty(), "API 失敗應回空集合：{set:?}");
        assert!(is_trading_day(day(2, 4), &set), "降級後平日都算交易日");
    }

    /// 回應的 stat 不是 OK 時視為沒有休市日資料。
    #[tokio::test]
    async fn get_holidays_set_ignores_non_ok_stat() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(HOLIDAY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "查詢日期大於今日",
                "date": "20190101",
                "queryYear": 2019,
                "total": 1,
                "data": [["2019-02-04", "農曆春節", "依規定放假"]]
            })))
            .mount(&server)
            .await;

        assert!(holidays(&server).await.is_empty());
    }
}

/// 以本機 mock server 取代證交所休市日表，以固定日期跑排程本體的非交易日路徑。
mod mock_holidays {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const TWSE: &str = "https://www.twse.com.tw";
    const ROUTE: &str = "/rwd/zh/holidaySchedule/holidaySchedule";

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("合法日期")
    }

    /// 回傳 `year` 年的休市日表，休市日為 `holidays`。
    async fn mount_year(server: &MockServer, year: i32, holidays: &[NaiveDate], expected: u64) {
        let data: Vec<serde_json::Value> = holidays
            .iter()
            .map(|day| serde_json::json!([day.format("%Y-%m-%d").to_string(), "測試休市", ""]))
            .collect();
        Mock::given(method("GET"))
            .and(path(ROUTE))
            .and(query_param("date", year.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stat": "ok", "queryYear": year, "total": data.len(), "data": data
            })))
            .expect(expected)
            .mount(server)
            .await;
    }

    /// 週末與交易所公告的休市日都直接略過（只讀休市日表，不派發提醒）。
    #[tokio::test]
    async fn run_skips_weekends_and_holidays() {
        let server = MockServer::start().await;
        // 1999-03-10（三）是休市日；03-13 是週六。兩次執行各讀一次當年度的休市日表。
        mount_year(&server, 1999, &[date(1999, 3, 10)], 2).await;
        let uri = server.uri();

        with_origins(&[(TWSE, &uri)], async {
            run(date(1999, 3, 13)).await.expect("週六略過");
            run(date(1999, 3, 10)).await.expect("休市日略過");
        })
        .await;
    }

    /// 12 月底往後 14 天會跨年：同時讀今年與明年的休市日表。
    #[tokio::test]
    async fn run_loads_next_year_when_horizon_crosses_new_year() {
        let server = MockServer::start().await;
        // 1999-12-27（一）設為休市日，讓本體在讀完兩年的表後直接略過。
        mount_year(&server, 1999, &[date(1999, 12, 27)], 1).await;
        mount_year(&server, 2000, &[], 1).await;
        let uri = server.uri();

        with_origins(&[(TWSE, &uri)], run(date(1999, 12, 27)))
            .await
            .expect("跨年略過");
    }

    /// 排程入口以今天執行：把今天（與 14 天後所在年度）設為休市日，不論哪天跑都只讀休市日表就結束。
    #[tokio::test]
    async fn execute_uses_today() {
        let today = Local::now().date_naive();
        let horizon = today + Days::new(HOLIDAY_HORIZON_DAYS);
        let server = MockServer::start().await;
        mount_year(&server, today.year(), &[today], 1).await;
        if horizon.year() != today.year() {
            mount_year(&server, horizon.year(), &[], 1).await;
        }
        let uri = server.uri();

        with_origins(&[(TWSE, &uri)], execute())
            .await
            .expect("今天休市略過");
    }
}
