use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};

use crate::{infra::crawler::twse};

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
            tracing::error!("Failed to fetch TWSE holiday schedule for {}, falling back to weekend check: {:?}",
                year, err);
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

/// 發送今日除權息提醒，並追加持股預估股利與下一交易日除權息預告通知。
///
/// 此任務每天早上 08:00 執行。現在會判斷今天是否為交易日：
/// 1. 若今天為非交易日，則直接跳過不處理，以避免重複或無效的通知。
/// 2. 若今天為交易日，則除原本今日提醒與持股計算外，會預報「下一個交易日」而非「曆法明天」的除權息名單。
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();

    // 載入今年度的休市日清單
    let current_year = today.year();
    let mut holidays = get_holidays_set(current_year).await;

    // 計算明天以判斷是否跨年，若跨年則需一併載入明年度的休市清單
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .context("Failed to calculate tomorrow for ex-dividend reminder")?;
    if tomorrow.year() != current_year {
        let next_year_holidays = get_holidays_set(tomorrow.year()).await;
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
        dotenv::dotenv().ok();
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
}
