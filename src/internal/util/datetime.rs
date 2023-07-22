use chrono::{DateTime, Local};

use crate::internal::logging;

/// A trait representing the weekend concept.
pub trait Weekend {
    /// Determines if a given date is a weekend.
    ///
    /// Returns `true` if the date is on a Saturday or Sunday, and `false` otherwise.
    fn is_weekend(&self) -> bool;
}

// Implement the `Weekend` trait for `chrono::DateTime<Local>`.
impl Weekend for DateTime<Local> {
    /// Treats Saturday and Sunday as weekends.
    ///
    /// This method checks if the given date falls on a Saturday or Sunday.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{Local, DateTime};
    /// use your_crate::Weekend;
    ///
    /// let date: DateTime<Local> = "2023-03-25T12:00:00".parse().unwrap();
    /// assert_eq!(date.is_weekend(), true);
    /// ```
    fn is_weekend(&self) -> bool {
        matches!(self.weekday(), Weekday::Sat | Weekday::Sun)
    }
}

/// Convert a month to its corresponding quarter.
///
/// The function accepts a `month` value, which is a `u32`, and returns
/// a static string slice representing the corresponding quarter. For example,
/// if the input month is 4, the function returns "Q2".
///
/// # Arguments
///
/// * `month` - A 32-bit unsigned integer representing a month (1-12)
///
/// # Examples
///
/// ```
/// let quarter = month_to_quarter(5);
/// assert_eq!(quarter, "Q2");
/// ```
///
/// # Panics
///
/// The function will not panic but returns "Invalid month" for any value outside of the valid range (1-12).
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
