use crate::logging;
use chrono::{DateTime, Datelike, Local, Weekday};

// 自定義的 Weekend trait
pub trait Weekend {
    fn is_weekend(&self) -> bool;
}

// 為 chrono::Date<Local> 實現 Weekend trait
impl Weekend for DateTime<Local> {
    /// 星期六、星期日視為假日
    fn is_weekend(&self) -> bool {
        matches!(self.weekday(), Weekday::Sat | Weekday::Sun)
    }
}

/// 月份轉季度
pub fn month_to_quarter(month: u32) -> &'static str {
    match month {
        1..=3 => "Q1",
        4..=6 => "Q2",
        7..=9 => "Q3",
        10..=12 => "Q4",
        _ => "Invalid month",
    }
}

/// Parses a date string in RFC 3339 format and returns a `DateTime<Local>`.
/// If the parsing fails, returns a default value of 1970-01-01T00:00:00Z.
///
/// # Arguments
///
/// * `date_str` - A date string in RFC 3339 format.
///
/// # Returns
///
/// A `DateTime<Local>` representing the parsed date or the default value.
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
