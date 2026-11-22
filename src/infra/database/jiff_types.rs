//! jiff 日期時間與 sqlx（PostgreSQL）的型別對應。
//!
//! sqlx 沒有內建 jiff 支援，改由 jiff 官方的 `jiff-sqlx` 提供包裝型別（`jiff_sqlx::Date` 等）：
//!
//! * 綁定參數：`query.bind(date.to_sqlx())`（需要 `jiff_sqlx::ToSqlx`）；`Option` 用
//!   `value.map(ToSqlx::to_sqlx)`。
//! * `FromRow` 欄位：非 `Option` 的 `jiff::civil::Date` 標 `#[sqlx(try_from = "jiff_sqlx::Date")]`；
//!   `Option<Date>` 標 `#[sqlx(try_from = "Nullable<jiff_sqlx::Date>")]`。
//! * [`TaipeiDateTime`]（`timestamptz`）本身就實作了 sqlx 的型別特徵，直接綁定與讀取即可，
//!   `Option<TaipeiDateTime>` 也一樣。

use jiff::civil::{Date, DateTime, Time};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgHasArrayType, PgTypeInfo, PgValueRef};
use sqlx::{Decode, Encode, Postgres, Type, ValueRef};

use crate::core::util::datetime::TaipeiDateTime;

impl Type<Postgres> for TaipeiDateTime {
    fn type_info() -> PgTypeInfo {
        <jiff_sqlx::Timestamp as Type<Postgres>>::type_info()
    }
}

impl PgHasArrayType for TaipeiDateTime {
    fn array_type_info() -> PgTypeInfo {
        <jiff_sqlx::Timestamp as PgHasArrayType>::array_type_info()
    }
}

impl Encode<'_, Postgres> for TaipeiDateTime {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        Encode::<Postgres>::encode(jiff_sqlx::Timestamp::from(self.timestamp()), buf)
    }
}

impl<'r> Decode<'r, Postgres> for TaipeiDateTime {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let timestamp = <jiff_sqlx::Timestamp as Decode<Postgres>>::decode(value)?;
        Ok(Self::from_timestamp(timestamp.to_jiff()))
    }
}

/// 可為 NULL 的欄位包裝，讓 `FromRow` 的 `try_from` 能把 `jiff_sqlx` 型別轉成 `Option<jiff 型別>`。
///
/// sqlx 的 `try_from` 要求欄位型別實作 `TryFrom<來源型別>`；`Option<Date>` 與
/// `Option<jiff_sqlx::Date>` 都不是本 crate 的型別，無法直接實作，因此以這個型別當來源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nullable<T>(pub Option<T>);

impl<T: Type<Postgres>> Type<Postgres> for Nullable<T> {
    fn type_info() -> PgTypeInfo {
        T::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        T::compatible(ty)
    }
}

impl<'r, T: Decode<'r, Postgres>> Decode<'r, Postgres> for Nullable<T> {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        if value.is_null() {
            return Ok(Self(None));
        }
        Ok(Self(Some(T::decode(value)?)))
    }
}

impl From<Nullable<jiff_sqlx::Date>> for Option<Date> {
    fn from(value: Nullable<jiff_sqlx::Date>) -> Self {
        value.0.map(jiff_sqlx::Date::to_jiff)
    }
}

impl From<Nullable<jiff_sqlx::DateTime>> for Option<DateTime> {
    fn from(value: Nullable<jiff_sqlx::DateTime>) -> Self {
        value.0.map(jiff_sqlx::DateTime::to_jiff)
    }
}

impl From<Nullable<jiff_sqlx::Time>> for Option<Time> {
    fn from(value: Nullable<jiff_sqlx::Time>) -> Self {
        value.0.map(jiff_sqlx::Time::to_jiff)
    }
}

#[cfg(test)]
mod tests;
