//! 股利預估誤差：把某次快照（`dividend_forecast_snapshot`）的逐筆預估與實際入帳比對。
//!
//! 預估的除息日與發放日可能是推估的，不一定和實際日期同一天，所以依（代號, 發放月份）配對，
//! 與 T23 手動比對腳本相同。發放日還沒到的預估列為待定（`pending`），不計入誤差。

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use crate::domain::portfolio::payout::PayoutAllocation;
use crate::domain::quote::peer::median;

/// 快照中的一筆預估。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastedPayout {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 預估發放日。
    pub pay_date: NaiveDate,
    /// 預估入帳金額。
    pub amount: Decimal,
    /// 預估依據（`announced`、`estimated` 等）。
    pub basis: String,
}

/// 一筆已入帳的現金股利。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActualPayout {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 發放日。
    pub pay_date: NaiveDate,
    /// 入帳金額。
    pub amount: Decimal,
}

/// 單一（代號, 發放月份）的比對結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccuracyStatus {
    /// 有預估也有入帳。
    Matched,
    /// 發放日還沒到。
    Pending,
    /// 有預估，但發放月份已過仍沒有入帳。
    Missed,
    /// 有入帳，但快照沒有預估（例如快照後才買進）。
    Unforecast,
}

impl AccuracyStatus {
    /// API 使用的 snake_case 名稱。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Pending => "pending",
            Self::Missed => "missed",
            Self::Unforecast => "unforecast",
        }
    }
}

/// 單一（代號, 發放月份）的預估與實際。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccuracyItem {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 發放月份的第一天。
    pub month: NaiveDate,
    /// 預估依據；同月多筆不同依據時以 `/` 串接（依字母排序），沒有預估時為空字串。
    pub basis: String,
    /// 預估金額合計。
    pub forecast: Decimal,
    /// 實際入帳金額合計；發放日還沒到時為 `None`。
    pub actual: Option<Decimal>,
    /// 實際 − 預估；發放日還沒到時為 `None`。
    pub difference: Option<Decimal>,
    /// |實際 − 預估| ÷ 實際 × 100；只有 [`AccuracyStatus::Matched`] 才有值。
    pub error_percent: Option<Decimal>,
    /// 比對結果。
    pub status: AccuracyStatus,
}

/// 單一預估依據的誤差彙總（只算 [`AccuracyStatus::Matched`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasisAccuracy {
    /// 預估依據（與 [`AccuracyItem::basis`] 相同）。
    pub basis: String,
    /// 比對筆數。
    pub count: usize,
    /// 誤差百分比中位數。
    pub median_error_percent: Option<Decimal>,
}

/// 一次快照的預估誤差。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForecastAccuracy {
    /// 依發放月份、代號排序的比對結果。
    pub items: Vec<AccuracyItem>,
    /// 依預估依據彙總，依依據名稱排序。
    pub by_basis: Vec<BasisAccuracy>,
    /// 已結算（發放日已到）的預估合計。
    pub settled_forecast: Decimal,
    /// 已結算的實際入帳合計。
    pub settled_actual: Decimal,
}

/// 同一（代號, 月份）的預估與實際累計。
#[derive(Default)]
struct Bucket {
    name: String,
    bases: Vec<String>,
    forecast: Decimal,
    has_forecast: bool,
    last_pay_date: Option<NaiveDate>,
    actual: Decimal,
    has_actual: bool,
}

/// 從入帳明細取出已入帳的現金股利：排除以目前持股推算的、沒有現金或沒有發放日的列。
#[must_use]
pub fn actual_payouts(allocations: &[PayoutAllocation]) -> Vec<ActualPayout> {
    allocations
        .iter()
        .filter(|allocation| !allocation.estimated && allocation.cash_amount > Decimal::ZERO)
        .filter_map(|allocation| {
            Some(ActualPayout {
                stock_symbol: allocation.stock_symbol.clone(),
                name: allocation.name.clone(),
                pay_date: allocation.cash_payable?,
                amount: allocation.cash_amount,
            })
        })
        .collect()
}

/// 月份的第一天。
fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// 依（代號, 發放月份）比對預估與實際入帳；`today` 之後才發放的預估列為待定。
///
/// `actuals` 只應包含發放日不晚於 `today` 的入帳。
#[must_use]
pub fn compare(
    forecasts: &[ForecastedPayout],
    actuals: &[ActualPayout],
    today: NaiveDate,
) -> ForecastAccuracy {
    let mut buckets: BTreeMap<(NaiveDate, String), Bucket> = BTreeMap::new();
    for forecast in forecasts {
        let bucket = buckets
            .entry((
                month_start(forecast.pay_date),
                forecast.stock_symbol.clone(),
            ))
            .or_default();
        bucket.name.clone_from(&forecast.name);
        if !bucket.bases.contains(&forecast.basis) {
            bucket.bases.push(forecast.basis.clone());
        }
        bucket.forecast += forecast.amount;
        bucket.has_forecast = true;
        bucket.last_pay_date = bucket.last_pay_date.max(Some(forecast.pay_date));
    }
    for actual in actuals {
        let bucket = buckets
            .entry((month_start(actual.pay_date), actual.stock_symbol.clone()))
            .or_default();
        if bucket.name.is_empty() {
            bucket.name.clone_from(&actual.name);
        }
        bucket.actual += actual.amount;
        bucket.has_actual = true;
    }

    let mut items = Vec::with_capacity(buckets.len());
    let mut errors: BTreeMap<String, Vec<Decimal>> = BTreeMap::new();
    let mut settled_forecast = Decimal::ZERO;
    let mut settled_actual = Decimal::ZERO;
    for ((month, stock_symbol), mut bucket) in buckets {
        bucket.bases.sort();
        let basis = bucket.bases.join("/");
        let status = match (bucket.has_forecast, bucket.has_actual) {
            (true, true) => AccuracyStatus::Matched,
            (false, _) => AccuracyStatus::Unforecast,
            (true, false) if bucket.last_pay_date.is_some_and(|date| date > today) => {
                AccuracyStatus::Pending
            }
            (true, false) => AccuracyStatus::Missed,
        };
        let (actual, difference, error_percent) = match status {
            AccuracyStatus::Pending => (None, None, None),
            AccuracyStatus::Matched => {
                let error = (bucket.actual > Decimal::ZERO).then(|| {
                    ((bucket.actual - bucket.forecast).abs() / bucket.actual * Decimal::ONE_HUNDRED)
                        .round_dp(2)
                });
                if let Some(error) = error {
                    errors.entry(basis.clone()).or_default().push(error);
                }
                (
                    Some(bucket.actual),
                    Some(bucket.actual - bucket.forecast),
                    error,
                )
            }
            AccuracyStatus::Missed | AccuracyStatus::Unforecast => (
                Some(bucket.actual),
                Some(bucket.actual - bucket.forecast),
                None,
            ),
        };
        if status != AccuracyStatus::Pending {
            settled_forecast += bucket.forecast;
            settled_actual += bucket.actual;
        }
        items.push(AccuracyItem {
            stock_symbol,
            name: bucket.name,
            month,
            basis,
            forecast: bucket.forecast,
            actual,
            difference,
            error_percent,
            status,
        });
    }

    let by_basis = errors
        .into_iter()
        .map(|(basis, values)| BasisAccuracy {
            basis,
            count: values.len(),
            median_error_percent: median(&values),
        })
        .collect();
    ForecastAccuracy {
        items,
        by_basis,
        settled_forecast,
        settled_actual,
    }
}

#[cfg(test)]
mod tests;
