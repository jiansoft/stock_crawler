//! 訊號績效與其他統計端點共用的參數驗證：期間、`group_by`、相對強度區間與連續買超天數。

use chrono::NaiveDate;

use super::{DEFAULT_PERIOD_DAYS, MAX_ALL_SIGNALS_PERIOD_DAYS, MAX_PERIOD_DAYS};

/// `group_by`：省略為不拆分，`year` 為依年度拆分，其他值回錯誤訊息。
pub(in crate::interfaces::web::data_api::handlers) fn parse_group_by(
    value: Option<&str>,
) -> Result<bool, &'static str> {
    match value {
        None => Ok(false),
        Some("year") => Ok(true),
        Some(_) => Err("group_by 只支援 year"),
    }
}

/// 連續買超天數門檻的上限；連續天數入庫時預先計算、不限回看長度，與條件選股一致。
pub(in crate::interfaces::web::data_api::handlers) const MAX_STREAK_DAYS: u8 = 60;

/// 外資、投信連續買超天數下限各在 1–[`MAX_STREAK_DAYS`]。
pub(in crate::interfaces::web::data_api::handlers) fn validate_streaks(
    foreign: Option<u8>,
    trust: Option<u8>,
) -> Result<(), &'static str> {
    let in_range = |days: Option<u8>| days.is_none_or(|days| (1..=MAX_STREAK_DAYS).contains(&days));
    if !in_range(foreign) {
        return Err("min_foreign_streak 必須介於 1 至 60");
    }
    if !in_range(trust) {
        return Err("min_trust_streak 必須介於 1 至 60");
    }
    Ok(())
}

/// 相對強度上下限各在 1–99，且下限不大於上限。
pub(in crate::interfaces::web::data_api::handlers) fn validate_rs_range(
    min: Option<u8>,
    max: Option<u8>,
) -> Result<(), &'static str> {
    let in_range = |score: Option<u8>| score.is_none_or(|score| (1..=99).contains(&score));
    if !in_range(min) {
        return Err("min_rs_score 必須介於 1 至 99");
    }
    if !in_range(max) {
        return Err("max_rs_score 必須介於 1 至 99");
    }
    if min.zip(max).is_some_and(|(min, max)| min > max) {
        return Err("min_rs_score 不可大於 max_rs_score");
    }
    Ok(())
}

/// 起日：未指定為迄日往前 [`DEFAULT_PERIOD_DAYS`] 天；晚於迄日或期間超過上限回錯誤訊息。
pub(in crate::interfaces::web::data_api::handlers) fn resolve_period(
    from: Option<NaiveDate>,
    to: NaiveDate,
) -> Result<NaiveDate, &'static str> {
    let from = from.unwrap_or(to - chrono::Duration::days(DEFAULT_PERIOD_DAYS));
    if from > to {
        return Err("from 不可晚於 to");
    }
    if (to - from).num_days() >= MAX_PERIOD_DAYS {
        return Err("統計期間最長 3,653 天");
    }
    Ok(from)
}

/// 期間超過 [`MAX_ALL_SIGNALS_PERIOD_DAYS`] 時必須指定訊號來源。
pub(super) fn require_indicator_for_long_period(
    from: NaiveDate,
    to: NaiveDate,
    indicator: Option<&str>,
) -> Result<(), &'static str> {
    if indicator.is_none() && (to - from).num_days() >= MAX_ALL_SIGNALS_PERIOD_DAYS {
        return Err("統計期間超過 1,096 天時必須指定 indicator");
    }
    Ok(())
}
