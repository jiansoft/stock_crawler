//! 回測參數的驗證與解析：基本面門檻範圍、排除產業清單與回測期間。

use super::*;

/// 檢查基本面門檻的範圍（與 `/stocks/screen` 相同）；超出時回錯誤訊息。
pub(super) fn validate_fundamentals(params: &ScreenBacktestParams) -> Result<(), &'static str> {
    let within = |value: Option<f64>, min: f64, max: f64| {
        value.is_none_or(|value| value.is_finite() && (min..=max).contains(&value))
    };
    if !within(params.min_revenue_yoy_percent, -100.0, 10_000.0) {
        return Err("min_revenue_yoy_percent 必須介於 -100 至 10000");
    }
    if params
        .min_revenue_yoy_streak
        .is_some_and(|months| !(1..=12).contains(&months))
    {
        return Err("min_revenue_yoy_streak 必須介於 1 至 12");
    }
    if !within(params.min_revenue_yoy_3m_percent, -100.0, 10_000.0) {
        return Err("min_revenue_yoy_3m_percent 必須介於 -100 至 10000");
    }
    if !within(params.min_monthly_revenue, 0.0, 1_000_000_000_000.0) {
        return Err("min_monthly_revenue 必須介於 0 至 1000000000000");
    }
    if !within(params.min_eps, -10_000.0, 10_000.0) {
        return Err("min_eps 必須介於 -10000 至 10000");
    }
    if !within(params.min_roe_percent, -10_000.0, 10_000.0) {
        return Err("min_roe_percent 必須介於 -10000 至 10000");
    }
    if !within(params.min_gross_margin_percent, -10_000.0, 100.0) {
        return Err("min_gross_margin_percent 必須介於 -10000 至 100");
    }
    if !within(params.min_operating_margin_percent, -10_000.0, 100.0) {
        return Err("min_operating_margin_percent 必須介於 -10000 至 100");
    }
    if !within(params.max_debt_ratio_percent, 0.0, 1_000.0) {
        return Err("max_debt_ratio_percent 必須介於 0 至 1000");
    }
    if !within(params.max_pe, 0.0, 10_000.0) {
        return Err("max_pe 必須介於 0 至 10000");
    }
    if !within(params.max_pb, 0.0, 1_000.0) {
        return Err("max_pb 必須介於 0 至 1000");
    }
    if !within(params.min_major_change, -100.0, 100.0) {
        return Err("min_major_change 必須介於 -100 至 100");
    }
    if params
        .major_change_weeks
        .is_some_and(|weeks| !(1..=12).contains(&weeks))
    {
        return Err("major_change_weeks 必須介於 1 至 12");
    }
    Ok(())
}

/// 設 `min_major_change` 時，起日要晚於大戶資料第一週（`first_week`，正式庫為 2023-07-14）加回看週數，
/// 第一期才有兩週可比；不足或沒有任何大戶資料時回錯誤訊息，不靜默略過條件。
pub(super) fn validate_major_change_period(
    params: &ScreenBacktestParams,
    from: NaiveDate,
    first_week: Option<NaiveDate>,
) -> Result<(), String> {
    if params.min_major_change.is_none() {
        return Ok(());
    }
    let Some(first_week) = first_week else {
        return Err("設定 min_major_change 時需要集保大戶資料，但資料庫沒有任何一週".to_owned());
    };
    let weeks = params
        .major_change_weeks
        .unwrap_or(super::super::DEFAULT_MAJOR_CHANGE_WEEKS);
    let earliest = first_week + chrono::Duration::weeks(i64::from(weeks));
    if from <= earliest {
        return Err(format!(
            "設定 min_major_change 時 from 必須晚於 {earliest}（集保大戶資料自 {first_week} 起，往後 {weeks} 週才有可比的兩週）"
        ));
    }
    Ok(())
}

/// 解析 `exclude_industry`：逗號分隔、去掉前後空白與空項目、去除重複；超過
/// [`MAX_EXCLUDED_INDUSTRIES`] 個回錯誤訊息。
pub(super) fn parse_excluded_industries(value: Option<&str>) -> Result<Vec<String>, &'static str> {
    let mut names: Vec<String> = Vec::new();
    for name in value.unwrap_or_default().split(',').map(str::trim) {
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_owned());
        }
    }
    if names.len() > MAX_EXCLUDED_INDUSTRIES {
        return Err("exclude_industry 最多 10 個產業");
    }
    Ok(names)
}

/// 起日：未指定為迄日往前 [`DEFAULT_PERIOD_DAYS`] 天；晚於迄日或期間超過上限回錯誤訊息。
pub(super) fn resolve_backtest_period(
    from: Option<NaiveDate>,
    to: NaiveDate,
) -> Result<NaiveDate, &'static str> {
    let from = from.unwrap_or(to - chrono::Duration::days(DEFAULT_PERIOD_DAYS));
    if from > to {
        return Err("from 不可晚於 to");
    }
    if (to - from).num_days() >= MAX_PERIOD_DAYS {
        return Err("回測期間最長 3,653 天");
    }
    Ok(from)
}
