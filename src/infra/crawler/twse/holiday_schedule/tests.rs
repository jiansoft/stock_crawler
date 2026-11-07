//! holiday_schedule的測試；連外的測試標 #[ignore]。

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

    /// 只備份去年以後的年度；測試程式一律不寫。
    #[test]
    fn only_recent_years_are_backed_up() {
        let year = Local::now().year();
        assert!(is_backed_up_year(year));
        assert!(is_backed_up_year(year - 1));
        assert!(!is_backed_up_year(year - 2));
        assert!(
            !should_back_up(year, &[holiday(year)]),
            "測試的 mock 休市日表不能寫進 Redis"
        );
    }

    /// 證交所讀不到、Redis 也沒有備份時回傳證交所的錯誤（1985 年從不寫備份）。
    #[tokio::test]
    async fn visit_or_backup_fails_without_backup() {
        let server = server("1985", broken_body()).await;
        let uri = server.uri();
        let result = with_origins(&[(TWSE, uri.as_str())], visit_or_backup(1985)).await;
        assert!(result.is_err(), "沒有備份時不能當成沒有休市日");
    }

    /// 連 Redis：證交所可讀時照常回傳；讀不到就讀先前寫入的備份。
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
        // 備份只寫去年以後的年度，用遠未來的年份避開真實資料。
        const YEAR: i32 = 2999;
        CLIENT.delete(&backup_key(YEAR)).await.expect("清掉舊備份");

        let ok = server("2999", ok_body(YEAR)).await;
        let uri = ok.uri();
        let fetched = with_origins(&[(TWSE, uri.as_str())], visit_or_backup(YEAR))
            .await
            .expect("證交所可讀");
        assert_eq!(fetched, vec![holiday(YEAR)]);
        assert_eq!(load_backup(YEAR).await, None, "測試程式不寫備份");
        save_backup(YEAR, &fetched).await;
        assert_eq!(load_backup(YEAR).await, Some(vec![holiday(YEAR)]));

        let broken = server("2999", broken_body()).await;
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
