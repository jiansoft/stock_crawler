//! 臺灣時間（固定 UTC+8）的日期時間工具，取代 chrono 的 `Local` 與 `DateTime<Local>`。
//!
//! 臺灣沒有夏令時間，固定 UTC+8 與 `Asia/Taipei` 結果相同，而且不依賴主機或容器的時區設定
//! 與 tzdata。

use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{self, TimeZone};
use jiff::{SignedDuration, Span, Timestamp, ZonedDateTime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 臺灣的 UTC 偏移（+08:00）。
const TAIPEI_OFFSET_HOURS: i8 = 8;

/// 回傳臺灣時區（固定 UTC+8）。
#[must_use]
pub fn taipei() -> TimeZone {
    TimeZone::fixed(tz::offset(TAIPEI_OFFSET_HOURS))
}

/// 回傳臺灣時間的今天。
#[must_use]
pub fn today() -> Date {
    TaipeiDateTime::now().date()
}

/// 以 chrono `NaiveDate::from_ymd_opt` 的參數型別建立日期；任一欄位超出範圍或日期不存在時回傳 `None`。
///
/// 年份範圍為 -9999～9999（jiff 的上限，chrono 可到 262143）。
#[must_use]
pub fn ymd(year: i32, month: u32, day: u32) -> Option<Date> {
    let year = i16::try_from(year).ok()?;
    let month = i8::try_from(month).ok()?;
    let day = i8::try_from(day).ok()?;
    Date::new(year, month, day).ok()
}

/// 臺灣時間的時間點，取代 `chrono::DateTime<Local>`。
///
/// 內部存 [`Timestamp`]；取年月日、時分等欄位時一律以臺灣時區（UTC+8）解讀。
/// 對外格式與 chrono 的 `DateTime<Local>` 在 UTC+8 主機上的輸出逐字相同：
///
/// * JSON（serde）與 [`fmt::Debug`]：`2026-11-22T13:05:09.123456+08:00`
/// * [`fmt::Display`]：`2026-11-22 13:05:09.123456 +08:00`
///
/// 秒以下的位數依 chrono 的規則：沒有小數時不輸出，否則補足 3、6 或 9 位。
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaipeiDateTime(Timestamp);

impl TaipeiDateTime {
    /// 現在的時間點。
    #[must_use]
    pub fn now() -> Self {
        Self(Timestamp::now())
    }

    /// 由時間點建立。
    #[must_use]
    pub const fn from_timestamp(timestamp: Timestamp) -> Self {
        Self(timestamp)
    }

    /// 以臺灣時區解讀本地日期時間；`DateTime` 超出可表示範圍時回傳錯誤。
    ///
    /// # Errors
    ///
    /// 日期時間超出 [`Timestamp`] 可表示的範圍時回傳 [`jiff::Error`]。
    pub fn from_datetime(datetime: DateTime) -> Result<Self, jiff::Error> {
        Ok(Self(taipei().to_timestamp(datetime)?))
    }

    /// 底層的時間點。
    #[must_use]
    pub const fn timestamp(self) -> Timestamp {
        self.0
    }

    /// 以臺灣時區表示的 [`ZonedDateTime`]。
    #[must_use]
    pub fn to_zoned_datetime(self) -> ZonedDateTime {
        self.0.to_zoned_datetime(taipei())
    }

    /// 臺灣時間的本地日期時間（取代 chrono 的 `naive_local`）。
    #[must_use]
    pub fn datetime(self) -> DateTime {
        taipei().to_datetime(self.0)
    }

    /// 臺灣時間的日期（取代 chrono 的 `date_naive`）。
    #[must_use]
    pub fn date(self) -> Date {
        self.datetime().date()
    }

    /// 臺灣時間的時刻。
    #[must_use]
    pub fn time(self) -> Time {
        self.datetime().time()
    }

    /// 依 strftime 格式輸出臺灣時間（取代 chrono 的 `format`）；`%z`、`%:z` 輸出 `+0800`、`+08:00`。
    #[must_use]
    pub fn strftime<'f, F>(self, format: &'f F) -> jiff::fmt::strtime::Display<'f>
    where
        F: 'f + ?Sized + AsRef<[u8]>,
    {
        self.to_zoned_datetime().strftime(format)
    }

    /// 加上時間長度；超出可表示範圍時回傳錯誤。
    ///
    /// # Errors
    ///
    /// 結果超出 [`Timestamp`] 可表示的範圍時回傳 [`jiff::Error`]。
    pub fn checked_add(self, duration: SignedDuration) -> Result<Self, jiff::Error> {
        Ok(Self(self.0.checked_add(duration)?))
    }

    /// 以臺灣時間的日曆加上日、月等單位（例如「一個月後」）；超出可表示範圍時回傳錯誤。
    ///
    /// # Errors
    ///
    /// 結果超出可表示的範圍時回傳 [`jiff::Error`]。
    pub fn checked_add_span(self, span: Span) -> Result<Self, jiff::Error> {
        Ok(Self(
            self.to_zoned_datetime().checked_add(span)?.timestamp(),
        ))
    }

    /// 與 chrono 相同格式的字串：`separator` 分隔日期與時刻，`offset_separator` 放在偏移之前。
    fn write_chrono_style(
        self,
        f: &mut fmt::Formatter<'_>,
        separator: char,
        offset_separator: &str,
    ) -> fmt::Result {
        let datetime = self.datetime();
        write!(
            f,
            "{}{separator}{:02}:{:02}:{:02}{}{offset_separator}+08:00",
            datetime.date(),
            datetime.hour(),
            datetime.minute(),
            datetime.second(),
            ChronoFraction(datetime.subsec_nanosecond()),
        )
    }
}

/// chrono 的秒以下位數規則：0 不輸出，否則依可整除的程度輸出 3、6 或 9 位。
struct ChronoFraction(i32);

impl fmt::Display for ChronoFraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nanos = self.0;
        if nanos == 0 {
            Ok(())
        } else if nanos % 1_000_000 == 0 {
            write!(f, ".{:03}", nanos / 1_000_000)
        } else if nanos % 1_000 == 0 {
            write!(f, ".{:06}", nanos / 1_000)
        } else {
            write!(f, ".{nanos:09}")
        }
    }
}

impl From<Timestamp> for TaipeiDateTime {
    fn from(timestamp: Timestamp) -> Self {
        Self(timestamp)
    }
}

impl From<TaipeiDateTime> for Timestamp {
    fn from(value: TaipeiDateTime) -> Self {
        value.0
    }
}

impl fmt::Display for TaipeiDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_chrono_style(f, ' ', " ")
    }
}

impl fmt::Debug for TaipeiDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_chrono_style(f, 'T', "")
    }
}

impl FromStr for TaipeiDateTime {
    type Err = jiff::Error;

    /// 解析帶偏移的 RFC 3339／ISO 8601 時間字串（例如 `2026-11-22T13:00:00+08:00`、`…Z`）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse::<Timestamp>()?))
    }
}

impl Serialize for TaipeiDateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("{self:?}"))
    }
}

impl<'de> Deserialize<'de> for TaipeiDateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl Add<SignedDuration> for TaipeiDateTime {
    type Output = Self;

    /// # Panics
    ///
    /// 結果超出 [`Timestamp`] 可表示的範圍時 panic（與 chrono 的 `+` 相同）。
    fn add(self, rhs: SignedDuration) -> Self {
        Self(self.0 + rhs)
    }
}

impl AddAssign<SignedDuration> for TaipeiDateTime {
    fn add_assign(&mut self, rhs: SignedDuration) {
        *self = *self + rhs;
    }
}

impl Sub<SignedDuration> for TaipeiDateTime {
    type Output = Self;

    /// # Panics
    ///
    /// 結果超出 [`Timestamp`] 可表示的範圍時 panic（與 chrono 的 `-` 相同）。
    fn sub(self, rhs: SignedDuration) -> Self {
        Self(self.0 - rhs)
    }
}

impl SubAssign<SignedDuration> for TaipeiDateTime {
    fn sub_assign(&mut self, rhs: SignedDuration) {
        *self = *self - rhs;
    }
}

impl Sub for TaipeiDateTime {
    type Output = SignedDuration;

    fn sub(self, rhs: Self) -> SignedDuration {
        self.0.duration_since(rhs.0)
    }
}

#[cfg(test)]
mod tests;
