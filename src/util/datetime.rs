use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta, Weekday};

use crate::declare::Quarter;
use crate::logging;

/// 提供「是否為週末」判斷能力的 trait。
///
/// 目前專門實作於 [`chrono::DateTime<Local>`]，用來統一專案內
/// 對六、日的判定方式。
pub trait Weekend {
    /// 判斷日期是否落在週末。
    ///
    /// 回傳 `true` 表示該日期是星期六或星期日，否則回傳 `false`。
    fn is_weekend(&self) -> bool;
}

impl Weekend for DateTime<Local> {
    /// 將星期六與星期日視為週末。
    ///
    /// 此實作僅依星期判斷，不處理國定假日、補班日或其他交易所休市規則。
    ///
    /// # 範例
    ///
    /// ```ignore
    /// use chrono::{DateTime, Local};
    /// use stock_crawler::util::datetime::Weekend;
    ///
    /// let date: DateTime<Local> = "2023-03-25T12:00:00".parse().unwrap();
    /// assert!(date.is_weekend());
    /// ```
    fn is_weekend(&self) -> bool {
        matches!(self.weekday(), Weekday::Sat | Weekday::Sun)
    }
}

/// 表示某個財報年度與季度的定位結果。
///
/// 這個結構用於描述「截至某一個日期為止，理論上應該已經可以完整取得的
/// 最新季報」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportQuarter {
    /// 季報所屬的西元年。
    pub year: i32,
    /// 季報所屬的季度。
    pub quarter: Quarter,
}

impl ReportQuarter {
    /// 建立一個財報季度定位結果。
    pub const fn new(year: i32, quarter: Quarter) -> Self {
        Self { year, quarter }
    }

    /// 回傳下一個曆法季度。
    ///
    /// 若目前為 `Q4`，則會自動進位到下一個年度的 `Q1`。
    pub const fn next(self) -> Self {
        match self.quarter {
            Quarter::Q1 => Self::new(self.year, Quarter::Q2),
            Quarter::Q2 => Self::new(self.year, Quarter::Q3),
            Quarter::Q3 => Self::new(self.year, Quarter::Q4),
            Quarter::Q4 => Self::new(self.year + 1, Quarter::Q1),
        }
    }
}

/// Yahoo 補欄位流程提前預抓下一季財報的預設觀察視窗天數。
///
/// 例如 `Q4` 法定截止日為 `3/31`，當日期進入截止日前 30 天內時，
/// 系統除了正式目標季度外，也會額外嘗試預抓下一季。
const BACKFILL_PRELOAD_DAYS_BEFORE_REPORT_DEADLINE: i64 = 30;

/// 依月份回傳對應的季度字串。
///
/// 這是一個簡單的月份轉季別工具，不會考慮財報公告時程，只根據月份本身
/// 所屬的曆法季度進行映射。
///
/// # 參數
///
/// * `month` - 月份，合法範圍為 `1..=12`
///
/// # 回傳值
///
/// * `Q1`、`Q2`、`Q3`、`Q4`
/// * 若月份超出範圍，回傳 `"Invalid month"`
///
/// # 範例
///
/// ```ignore
/// use stock_crawler::util::datetime::month_to_quarter;
///
/// assert_eq!(month_to_quarter(5), "Q2");
/// ```
pub fn month_to_quarter(month: u32) -> &'static str {
    match month {
        1..=3 => "Q1",
        4..=6 => "Q2",
        7..=9 => "Q3",
        10..=12 => "Q4",
        _ => "Invalid month",
    }
}

/// 依上市與上櫃公司季報法定申報截止日，推導目前可視為「已完整公告」的最新季別。
///
/// 這個函式不是在算「上一個曆法季度」，而是依據上市/上櫃公司常見的季報公告
/// 截止日來決定目前應該追蹤哪一期季報：
///
/// * 前一年度 `Q4`：`3/31` 截止，因此自 `4/1` 起視為可完整取得
/// * 當年度 `Q1`：`5/15` 截止，因此自 `5/16` 起視為可完整取得
/// * 當年度 `Q2`：`8/14` 截止，因此自 `8/15` 起視為可完整取得
/// * 當年度 `Q3`：`11/14` 截止，因此自 `11/15` 起視為可完整取得
///
/// 因此在每年 `1/1` 到 `3/31` 之間，最新可完整取得的季報仍視為前一年度 `Q3`。
/// 這樣可以避免在法定截止日前，過早切換到尚未全部公告完成的季度。
///
/// # 參數
///
/// * `now` - 用來判斷目標季別的當地時間
///
/// # 回傳值
///
/// 回傳 [`ReportQuarter`]，包含目標季報的西元年與季別。
///
/// # 範例
///
/// ```ignore
/// use chrono::Local;
/// use stock_crawler::util::datetime::latest_published_quarter_for_listed_and_otc;
///
/// let report_quarter = latest_published_quarter_for_listed_and_otc(Local::now());
/// println!("{} {}", report_quarter.year, report_quarter.quarter);
/// ```
pub fn latest_published_quarter_for_listed_and_otc(now: DateTime<Local>) -> ReportQuarter {
    latest_published_quarter_for_listed_and_otc_by_date(now.date_naive())
}

/// 取得上市/上櫃季 EPS 流程目前應處理的季度清單。
///
/// 回傳結果至少會包含一個「正式目標季度」；若目前日期已進入下一季的
/// 「季末隔天起」預抓視窗，則會再額外附上一個「預抓季度」。
///
/// 例如在 `2026-03-25`：
///
/// * 正式目標季度為 `2025 Q3`
/// * 由於 `2025 Q4` 已經在 `2025-12-31` 結束，且目前日期已超過季末
/// * 因此回傳會同時包含 `2025 Q3` 與 `2025 Q4`
///
/// 這樣可以在大多數公司提早上傳季報時，先行把已公告資料收進資料庫，
/// 等正式截止日過後再自然切換主目標季度。
pub fn eps_report_quarter_targets_for_listed_and_otc(now: DateTime<Local>) -> Vec<ReportQuarter> {
    eps_report_quarter_targets_for_listed_and_otc_by_date(now.date_naive())
}

/// 取得上市/上櫃 Yahoo 補欄位流程目前應處理的季度清單。
///
/// 此函式沿用較保守的預抓策略，只有在法定截止日前一段時間內，
/// 才會將下一季納入處理清單，避免 Yahoo 財務欄位來源過早查詢時雜訊過多。
pub fn backfill_report_quarter_targets_for_listed_and_otc(
    now: DateTime<Local>,
) -> Vec<ReportQuarter> {
    backfill_report_quarter_targets_for_listed_and_otc_by_date(now.date_naive())
}

/// 解析 RFC 3339 日期字串並轉成本地時區時間。
///
/// 若解析失敗，會記錄錯誤日誌，並回傳 `1970-01-01T00:00:00Z` 轉換為本地時區後
/// 的時間，讓呼叫端在無法拋錯的情境下仍能取得可預期的預設值。
///
/// # 參數
///
/// * `date_str` - RFC 3339 格式的日期字串
///
/// # 回傳值
///
/// 解析成功時回傳對應的 [`DateTime<Local>`]；失敗時回傳預設時間。
pub fn parse_date(date_str: &str) -> DateTime<Local> {
    match DateTime::parse_from_rfc3339(date_str) {
        Ok(dt) => dt.with_timezone(&Local),
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to parse date string '{}': {}",
                date_str, why
            ));
            DateTime::parse_from_rfc3339("1970-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Local)
        }
    }
}

/// 將民國年轉成西元年。
///
/// 計算方式為 `民國年 + 1911`。
pub fn roc_year_to_gregorian_year(roc_year: i32) -> i32 {
    roc_year + 1911
}

/// 將西元年轉成民國年。
///
/// 計算方式為 `西元年 - 1911`。
pub fn gregorian_year_to_roc_year(gregorian_year: i32) -> i32 {
    gregorian_year - 1911
}

/// 解析民國日期字串並轉成西元曆的 [`NaiveDate`]。
///
/// 此函式接受 `YYY/MM/DD` 或 `YYY-MM-DD` 格式的民國日期，例如 `113/08/14`，
/// 並將年份轉成西元後回傳。
///
/// 若字串格式不正確、任一欄位無法解析，或日期本身不存在，則回傳 `None`。
pub fn parse_taiwan_date(date_str: &str) -> Option<NaiveDate> {
    let split_date: Vec<&str> = date_str.split(['/', '-']).collect();
    if split_date.len() != 3 {
        return None;
    }

    let year = roc_year_to_gregorian_year(parse_date_part::<i32>(split_date[0])?);
    let month = parse_date_part::<u32>(split_date[1])?;
    let day = parse_date_part::<u32>(split_date[2])?;

    NaiveDate::from_ymd_opt(year, month, day)
}

/// 解析短格式民國日期字串（無分隔符號）並轉成西元曆的 [`NaiveDate`]。
///
/// 此函式專門處理台灣證券交易所（TWSE）OpenAPI 格式中常見的 `YYYMMDD` 字串。
/// 例如：`1150409` 代表民國 115 年 4 月 9 日，會被轉換為 `2026-04-09`。
///
/// # 參數
///
/// * `date_str` - 7 位數的民國日期字串，格式必須為 `YYYMMDD`。
///
/// # 回傳值
///
/// * `Some(NaiveDate)` - 解析成功且日期合法。
/// * `None` - 字串長度不符（非 7 位）、內容包含非數字，或日期邏輯不合法（如 2 月 30 日）。
///
/// # 範例
///
/// ```ignore
/// use stock_crawler::util::datetime::parse_taiwan_date_short;
///
/// let date = parse_taiwan_date_short("1150409").unwrap();
/// assert_eq!(date.to_string(), "2026-04-09");
/// ```
pub fn parse_taiwan_date_short(date_str: &str) -> Option<NaiveDate> {
    if date_str.len() != 7 {
        return None;
    }

    let year_str = &date_str[0..3];
    let month_str = &date_str[3..5];
    let day_str = &date_str[5..7];

    let year = roc_year_to_gregorian_year(parse_date_part::<i32>(year_str)?);
    let month = parse_date_part::<u32>(month_str)?;
    let day = parse_date_part::<u32>(day_str)?;

    NaiveDate::from_ymd_opt(year, month, day)
}

/// 依上市/上櫃公司季報法定申報截止日，計算指定日期對應的最新已公告季別。
fn latest_published_quarter_for_listed_and_otc_by_date(current_date: NaiveDate) -> ReportQuarter {
    let year = current_date.year();
    let q4_deadline = NaiveDate::from_ymd_opt(year, 3, 31).expect("invalid Q4 deadline");
    let q1_deadline = NaiveDate::from_ymd_opt(year, 5, 15).expect("invalid Q1 deadline");
    let q2_deadline = NaiveDate::from_ymd_opt(year, 8, 14).expect("invalid Q2 deadline");
    let q3_deadline = NaiveDate::from_ymd_opt(year, 11, 14).expect("invalid Q3 deadline");

    if current_date > q3_deadline {
        ReportQuarter {
            year,
            quarter: Quarter::Q3,
        }
    } else if current_date > q2_deadline {
        ReportQuarter {
            year,
            quarter: Quarter::Q2,
        }
    } else if current_date > q1_deadline {
        ReportQuarter {
            year,
            quarter: Quarter::Q1,
        }
    } else if current_date > q4_deadline {
        ReportQuarter {
            year: year - 1,
            quarter: Quarter::Q4,
        }
    } else {
        ReportQuarter {
            year: year - 1,
            quarter: Quarter::Q3,
        }
    }
}

/// 依目前日期回傳季 EPS 流程的正式季度與可能的預抓季度。
fn eps_report_quarter_targets_for_listed_and_otc_by_date(
    current_date: NaiveDate,
) -> Vec<ReportQuarter> {
    let published = latest_published_quarter_for_listed_and_otc_by_date(current_date);
    let mut targets = vec![published];
    let preload_quarter = published.next();

    if should_preload_next_quarter_for_eps(current_date, preload_quarter) {
        targets.push(preload_quarter);
    }

    targets
}

/// 依目前日期回傳 Yahoo 補欄位流程的正式季度與可能的預抓季度。
fn backfill_report_quarter_targets_for_listed_and_otc_by_date(
    current_date: NaiveDate,
) -> Vec<ReportQuarter> {
    let published = latest_published_quarter_for_listed_and_otc_by_date(current_date);
    let mut targets = vec![published];
    let preload_quarter = published.next();

    if should_preload_next_quarter_for_backfill(current_date, preload_quarter) {
        targets.push(preload_quarter);
    }

    targets
}

/// 判斷目前是否已進入季 EPS 流程的預抓視窗。
///
/// 規則是「只要季末已過，就開始預抓下一季」。
fn should_preload_next_quarter_for_eps(
    current_date: NaiveDate,
    preload_quarter: ReportQuarter,
) -> bool {
    current_date >= report_period_end(preload_quarter) + TimeDelta::try_days(1).unwrap()
}

/// 判斷目前是否已進入 Yahoo 補欄位流程的預抓視窗。
fn should_preload_next_quarter_for_backfill(
    current_date: NaiveDate,
    preload_quarter: ReportQuarter,
) -> bool {
    let deadline = report_deadline_for_listed_and_otc(preload_quarter);
    let preload_start =
        deadline - TimeDelta::try_days(BACKFILL_PRELOAD_DAYS_BEFORE_REPORT_DEADLINE).unwrap();

    current_date >= preload_start && current_date <= deadline
}

/// 回傳指定季度的季度結束日。
fn report_period_end(report_quarter: ReportQuarter) -> NaiveDate {
    match report_quarter.quarter {
        Quarter::Q1 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 3, 31).expect("invalid Q1 period end")
        }
        Quarter::Q2 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 6, 30).expect("invalid Q2 period end")
        }
        Quarter::Q3 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 9, 30).expect("invalid Q3 period end")
        }
        Quarter::Q4 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 12, 31).expect("invalid Q4 period end")
        }
    }
}

/// 回傳指定財報季度對應的法定申報截止日。
fn report_deadline_for_listed_and_otc(report_quarter: ReportQuarter) -> NaiveDate {
    match report_quarter.quarter {
        Quarter::Q1 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 5, 15).expect("invalid Q1 deadline")
        }
        Quarter::Q2 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 8, 14).expect("invalid Q2 deadline")
        }
        Quarter::Q3 => {
            NaiveDate::from_ymd_opt(report_quarter.year, 11, 14).expect("invalid Q3 deadline")
        }
        Quarter::Q4 => {
            NaiveDate::from_ymd_opt(report_quarter.year + 1, 3, 31).expect("invalid Q4 deadline")
        }
    }
}

/// 嘗試解析日期片段。
///
/// 這是 [`parse_taiwan_date`] 的內部輔助函式，若字串無法轉成指定型別則回傳 `None`。
fn parse_date_part<T: std::str::FromStr>(date_part_str: &str) -> Option<T> {
    date_part_str.parse::<T>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驗證上市/上櫃最新已公告季別在各截止日邊界的切換是否正確。
    #[test]
    fn test_latest_published_quarter_for_listed_and_otc_by_date() {
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()
            ),
            ReportQuarter::new(2025, Quarter::Q3)
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
            ),
            ReportQuarter::new(2025, Quarter::Q4)
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 5, 15).unwrap()
            ),
            ReportQuarter::new(2025, Quarter::Q4)
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 5, 16).unwrap()
            ),
            ReportQuarter::new(2026, Quarter::Q1)
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 8, 15).unwrap()
            ),
            ReportQuarter::new(2026, Quarter::Q2)
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
            ),
            ReportQuarter::new(2026, Quarter::Q3)
        );
    }

    /// 驗證季 EPS 流程在季末後會開始預抓下一季。
    #[test]
    fn test_eps_report_quarter_targets_for_listed_and_otc_by_date() {
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()
            ),
            vec![ReportQuarter::new(2025, Quarter::Q3)]
        );
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q3),
                ReportQuarter::new(2025, Quarter::Q4),
            ]
        );
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q3),
                ReportQuarter::new(2025, Quarter::Q4),
            ]
        );
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q3),
                ReportQuarter::new(2025, Quarter::Q4),
            ]
        );
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 3, 25).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q3),
                ReportQuarter::new(2025, Quarter::Q4),
            ]
        );
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q4),
                ReportQuarter::new(2026, Quarter::Q1),
            ]
        );
        assert_eq!(
            eps_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q4),
                ReportQuarter::new(2026, Quarter::Q1),
            ]
        );
    }

    /// 驗證 Yahoo 補欄位流程仍維持較保守的截止日前預抓視窗。
    #[test]
    fn test_backfill_report_quarter_targets_for_listed_and_otc_by_date() {
        assert_eq!(
            backfill_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
            ),
            vec![ReportQuarter::new(2025, Quarter::Q3)]
        );
        assert_eq!(
            backfill_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q3),
                ReportQuarter::new(2025, Quarter::Q4),
            ]
        );
        assert_eq!(
            backfill_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
            ),
            vec![ReportQuarter::new(2025, Quarter::Q4)]
        );
        assert_eq!(
            backfill_report_quarter_targets_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
            ),
            vec![
                ReportQuarter::new(2025, Quarter::Q4),
                ReportQuarter::new(2026, Quarter::Q1),
            ]
        );
    }
}
