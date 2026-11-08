//! `/market/dividend-calendar` handler、其 UNION ALL 參數化 SQL、的測試。

//! 行事曆查詢區間解析的 deterministic tests。

use chrono::NaiveDate;

use super::resolve_calendar_range;

/// §4.9 行事曆區間解析：預設值、區間顛倒、92 天上限與溢位防護
/// 都必須是 deterministic 純函式行為。
#[test]
fn calendar_range_resolution_follows_section_4_9() {
    let today = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
    // 兩者皆缺省：from = 當日、to = from + 30 天。
    assert_eq!(
        resolve_calendar_range(None, None, today),
        Ok((today, NaiveDate::from_ymd_opt(2026, 8, 16).unwrap()))
    );
    // 只給 from：to 以 from（而非當日）為基準加 30 天。
    let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    assert_eq!(
        resolve_calendar_range(Some(from), None, today),
        Ok((from, NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()))
    );
    // 區間顛倒與超界：差 92 天是最後一個合法值，93 天必須被拒。
    let to_92 = NaiveDate::from_ymd_opt(2026, 4, 3).unwrap();
    let to_93 = NaiveDate::from_ymd_opt(2026, 4, 4).unwrap();
    assert_eq!(
        resolve_calendar_range(Some(from), Some(to_92), today)
            .unwrap()
            .1,
        to_92
    );
    assert_eq!(
        resolve_calendar_range(Some(from), Some(to_93), today),
        Err("查詢區間不可超過 92 天")
    );
    assert_eq!(
        resolve_calendar_range(Some(to_92), Some(from), today),
        Err("from 不可晚於 to")
    );
    // 極端日期的 to 預設值不可 panic，必須轉成參數錯誤。chrono 的
    // `NaiveDate` 值域超過西元 9999（`parse_optional_date` 的十字元
    // 格式實際到不了 `NaiveDate::MAX`），此斷言驗證 guard 讓函式對
    // 全值域皆為 total function，不留下 panic 路徑。
    assert_eq!(
        resolve_calendar_range(Some(NaiveDate::MAX), None, today),
        Err("日期超出可查詢範圍")
    );
}
