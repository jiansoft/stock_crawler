use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};

use crate::infra::crawler::twse;

mod preview;

/// 載入休市日要涵蓋今天之後幾天：下一交易日與持股預告窗口跨年時一併載入明年。
const HOLIDAY_HORIZON_DAYS: u64 = 14;

/// 取得指定年份的交易所休市日集合。
///
/// 此函式會呼叫台灣證券交易所（TWSE）的休市日 API，將回傳的日期放入 `HashSet` 中。
/// 若連線或解析失敗，會透過 `logging::error_file_async` 記錄錯誤，並回傳空集合，
/// 此時系統將會降級為僅依據星期六、日進行交易日判斷。
///
/// # 參數
///
/// * `year` - 要查詢的西元年份。
async fn get_holidays_set(year: i32) -> HashSet<NaiveDate> {
    // 呼叫 TWSE 的休市日 API 取得資料
    match twse::holiday_schedule::visit(year).await {
        Ok(holidays) => {
            // 將所有休市日期收集至 HashSet 以利後續快速查表
            holidays.into_iter().map(|h| h.date).collect()
        }
        Err(err) => {
            // 發生網路或解析錯誤時，發送錯誤日誌並降級（回傳空集合）
            tracing::error!(
                "Failed to fetch TWSE holiday schedule for {}, falling back to weekend check: {:?}",
                year,
                err
            );
            HashSet::new()
        }
    }
}

/// 判斷特定日期是否為交易日。
///
/// 若該日期為星期六或星期日，或是存在於 `holidays` 休市日集合中，則判定為非交易日。
///
/// # 參數
///
/// * `date` - 要判定的日期。
/// * `holidays` - 已載入的休市日 `HashSet`。
fn is_trading_day(date: NaiveDate, holidays: &HashSet<NaiveDate>) -> bool {
    // 1. 如果是星期六或星期日，則不是交易日
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    // 2. 如果該日期在交易所公告的休市日名單中，則不是交易日
    !holidays.contains(&date)
}

/// 尋找指定日期之後的下一個交易日。
///
/// 從指定日期的隔天開始遞增，直到找到符合交易日條件的日期為止。
///
/// # 參數
///
/// * `date` - 起始日期。
/// * `holidays` - 已載入的休市日 `HashSet`。
fn find_next_trading_day(mut date: NaiveDate, holidays: &HashSet<NaiveDate>) -> NaiveDate {
    loop {
        // 遞增一天，若加天數溢出則回退使用一般的加法運算
        date = date
            .checked_add_days(Days::new(1))
            .unwrap_or(date + Days::new(1));
        // 判斷遞增後的日期是否為交易日
        if is_trading_day(date, holidays) {
            return date;
        }
    }
}

/// 發送今日除權息提醒，並追加持股預估股利、下一交易日除權息預告與持股除權息預告通知。
///
/// 此任務每天早上 08:00 執行。現在會判斷今天是否為交易日：
/// 1. 若今天為非交易日，則直接跳過不處理，以避免重複或無效的通知。
/// 2. 若今天為交易日，則除原本今日提醒與持股計算外，會預報「下一個交易日」而非「曆法明天」的除權息名單。
/// 3. 持股在今天之後 3 個交易日內除權息的，另發一則預告（見 [`preview`]），每檔只預告一次。
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();

    // 載入今年度的休市日清單
    let current_year = today.year();
    let mut holidays = get_holidays_set(current_year).await;

    // 下一交易日與持股預告窗口可能跨年，跨年則需一併載入明年度的休市清單
    let horizon = today
        .checked_add_days(Days::new(HOLIDAY_HORIZON_DAYS))
        .context("Failed to calculate holiday horizon for ex-dividend reminder")?;
    if horizon.year() != current_year {
        let next_year_holidays = get_holidays_set(horizon.year()).await;
        holidays.extend(next_year_holidays);
    }

    // 若今天不是交易日（週末或節假日），直接跳過不發送通知
    if !is_trading_day(today, &holidays) {
        return Ok(());
    }

    // 尋找下一個實際交易日
    let next_trading = find_next_trading_day(today, &holidays);

    // 派發除權息提醒領域事件以進行非同步背景通知與記錄處理
    let dispatcher = crate::app::event::get_global_dispatcher();
    dispatcher
        .dispatch_async(vec![
            crate::domain::events::DomainEvent::ExDividendReminderTriggered {
                date: today,
                next_trading_date: next_trading,
                occurred_at: Local::now(),
            },
        ])
        .await;

    // 持股預告與市場清單互不影響：失敗只記錄，不讓排程回報錯誤
    if let Err(why) = preview::send(today, &holidays).await {
        tracing::error!("持股除權息預告失敗: {why:#}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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
}
