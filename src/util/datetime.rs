use chrono::{DateTime, Datelike, Local, NaiveDate, Weekday};

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
            ReportQuarter {
                year: 2025,
                quarter: Quarter::Q3,
            }
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
            ),
            ReportQuarter {
                year: 2025,
                quarter: Quarter::Q4,
            }
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 5, 15).unwrap()
            ),
            ReportQuarter {
                year: 2025,
                quarter: Quarter::Q4,
            }
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 5, 16).unwrap()
            ),
            ReportQuarter {
                year: 2026,
                quarter: Quarter::Q1,
            }
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 8, 15).unwrap()
            ),
            ReportQuarter {
                year: 2026,
                quarter: Quarter::Q2,
            }
        );
        assert_eq!(
            latest_published_quarter_for_listed_and_otc_by_date(
                NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
            ),
            ReportQuarter {
                year: 2026,
                quarter: Quarter::Q3,
            }
        );
    }
}
