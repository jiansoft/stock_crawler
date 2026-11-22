//! 以 UTC（`Z` 結尾）序列化 [`Timestamp`]，格式與 chrono 的 `DateTime<Utc>` 逐字相同。
//!
//! jiff 的 `Timestamp` 預設只輸出必要的小數位數（`.12`），chrono 依可整除的程度輸出 3、6 或
//! 9 位（`.120`）。原本是 `DateTime<Utc>` 的欄位標上 `#[serde(with = "…::rfc3339_utc")]`，
//! 讓 JSON 與快取內容不變。

use std::fmt;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Deserializer, Serializer};

use super::taipei_time::ChronoFraction;

/// 與 chrono `DateTime<Utc>` 相同的字串：`2026-11-22T05:05:09.123456Z`。
struct ChronoUtc(Timestamp);

impl fmt::Display for ChronoUtc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let datetime = TimeZone::UTC.to_datetime(self.0);
        write!(
            f,
            "{}T{:02}:{:02}:{:02}{}Z",
            datetime.date(),
            datetime.hour(),
            datetime.minute(),
            datetime.second(),
            ChronoFraction(datetime.subsec_nanosecond()),
        )
    }
}

/// 與 chrono `DateTime<Utc>::to_rfc3339()` 相同的字串：`2026-11-22T05:05:09.123456+00:00`。
#[must_use]
pub fn to_rfc3339(value: Timestamp) -> String {
    let text = ChronoUtc(value).to_string();
    format!("{}+00:00", text.trim_end_matches('Z'))
}

/// serde 序列化。
///
/// # Errors
///
/// 序列化器寫入失敗時回傳錯誤。
pub fn serialize<S: Serializer>(value: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&ChronoUtc(*value))
}

/// serde 還原：接受帶任何偏移的 RFC 3339 字串。
///
/// # Errors
///
/// 字串不是合法的時間點時回傳錯誤。
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
    let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
    text.parse().map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests;
