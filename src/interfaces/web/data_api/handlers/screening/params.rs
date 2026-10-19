//! 條件選股的參數驗證與排序白名單。

#[cfg(test)]
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::super::market_id_for_stocks;
use crate::interfaces::web::data_api::dto::StockScreeningParams;

/// 驗證後可安全綁定條件選股 SQL 的內部參數。
///
/// 數值門檻預先轉為 `Decimal`，使 PostgreSQL 能以原生 `NUMERIC` 比較而不會
/// 因浮點參數產生隱含型別轉換；排序則只保留程式內建的靜態 SQL 片段。
pub(super) struct ValidatedScreeningParams {
    /// 股票表市場 id；0 只代表 SQL 內固定展開的上市加上櫃。
    pub(super) market_id: i32,
    /// 最低營收年增率。
    pub(super) min_revenue_yoy_percent: Option<Decimal>,
    /// 最低近 3 個月累計營收年增率。
    pub(super) min_revenue_yoy_3m_percent: Option<Decimal>,
    /// 最新一個月營收下限（千元）。
    pub(super) min_monthly_revenue: Option<Decimal>,
    /// 最低每股盈餘。
    pub(super) min_eps: Option<Decimal>,
    /// 最低股東權益報酬率。
    pub(super) min_roe_percent: Option<Decimal>,
    /// 最低殖利率。
    pub(super) min_dividend_yield_percent: Option<Decimal>,
    /// 最低單季毛利率。
    pub(super) min_gross_margin_percent: Option<Decimal>,
    /// 最高季末負債比。
    pub(super) max_debt_ratio_percent: Option<Decimal>,
    /// 最低單季營業利益率。
    pub(super) min_operating_margin_percent: Option<Decimal>,
    /// 最低相對強度分數。
    pub(super) min_rs_score: Option<i16>,
    /// 外資最少連續買超天數。
    pub(super) min_foreign_streak: Option<i16>,
    /// 投信最少連續買超天數。
    pub(super) min_trust_streak: Option<i16>,
    /// 近 20 筆日線平均成交金額下限（元）。
    pub(super) min_avg_trade_value: Option<Decimal>,
    /// 二十八個固定排序分支之一。
    pub(super) order_by: &'static str,
    /// 查詢筆數上限。
    pub(super) limit: u8,
}

/// 驗證條件選股參數並轉換成資料庫可直接綁定的型別。
///
/// `market=twse/tpex` 本身就是有效篩選；`market=all` 不縮小上市櫃集合，
/// 因此必須再提供產業、估值或至少一個數值門檻，避免 endpoint 被當成無條件
/// 的全市場匯出介面。
pub(super) fn validate_screening_params(
    params: &StockScreeningParams,
) -> Result<ValidatedScreeningParams, &'static str> {
    let market = params.market.as_deref().unwrap_or("all");
    let market_id = market_id_for_stocks(market).ok_or("market 必須為 all、twse 或 tpex")?;
    if params.industry_id.is_some_and(|value| value <= 0) {
        return Err("industry_id 必須為正整數");
    }
    if params.valuation_band.as_deref().is_some_and(|value| {
        ![
            "undervalued",
            "fair_valued",
            "overvalued",
            "highly_overvalued",
        ]
        .contains(&value)
    }) {
        return Err("valuation_band 不在允許範圍");
    }
    let min_revenue_yoy_percent = decimal_in_range(
        params.min_revenue_yoy_percent,
        -100.0,
        10_000.0,
        "min_revenue_yoy_percent 必須介於 -100 至 10000",
    )?;
    if params
        .min_revenue_yoy_streak
        .is_some_and(|months| !(1..=12).contains(&months))
    {
        return Err("min_revenue_yoy_streak 必須介於 1 至 12");
    }
    let min_revenue_yoy_3m_percent = decimal_in_range(
        params.min_revenue_yoy_3m_percent,
        -100.0,
        10_000.0,
        "min_revenue_yoy_3m_percent 必須介於 -100 至 10000",
    )?;
    let min_monthly_revenue = decimal_in_range(
        params.min_monthly_revenue,
        0.0,
        1_000_000_000_000.0,
        "min_monthly_revenue 必須介於 0 至 1000000000000",
    )?;
    let min_eps = decimal_in_range(
        params.min_eps,
        -10_000.0,
        10_000.0,
        "min_eps 必須介於 -10000 至 10000",
    )?;
    let min_roe_percent = decimal_in_range(
        params.min_roe_percent,
        -10_000.0,
        10_000.0,
        "min_roe_percent 必須介於 -10000 至 10000",
    )?;
    let min_dividend_yield_percent = decimal_in_range(
        params.min_dividend_yield_percent,
        0.0,
        1_000.0,
        "min_dividend_yield_percent 必須介於 0 至 1000",
    )?;
    if params.min_f_score.is_some_and(|score| score > 9) {
        return Err("min_f_score 必須介於 0 至 9");
    }
    let min_gross_margin_percent = decimal_in_range(
        params.min_gross_margin_percent,
        -10_000.0,
        100.0,
        "min_gross_margin_percent 必須介於 -10000 至 100",
    )?;
    let max_debt_ratio_percent = decimal_in_range(
        params.max_debt_ratio_percent,
        0.0,
        1_000.0,
        "max_debt_ratio_percent 必須介於 0 至 1000",
    )?;
    let min_operating_margin_percent = decimal_in_range(
        params.min_operating_margin_percent,
        -10_000.0,
        100.0,
        "min_operating_margin_percent 必須介於 -10000 至 100",
    )?;
    if params
        .min_rs_score
        .is_some_and(|score| !(1..=99).contains(&score))
    {
        return Err("min_rs_score 必須介於 1 至 99");
    }
    let streak_out_of_range = |days: Option<u8>| days.is_some_and(|days| !(1..=60).contains(&days));
    if streak_out_of_range(params.min_foreign_streak) {
        return Err("min_foreign_streak 必須介於 1 至 60");
    }
    if streak_out_of_range(params.min_trust_streak) {
        return Err("min_trust_streak 必須介於 1 至 60");
    }
    if params
        .min_avg_trade_value
        .is_some_and(|value| !(1..=100_000).contains(&value))
    {
        return Err("min_avg_trade_value 必須介於 1 至 100000（百萬元）");
    }
    let order_by = screen_order_by(
        params.sort_by.as_deref().unwrap_or("stock_symbol"),
        params.sort_order.as_deref().unwrap_or("asc"),
    )?;
    let limit = params.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err("limit 必須介於 1 至 50");
    }
    let has_filter = market != "all"
        || params.industry_id.is_some()
        || params.valuation_band.is_some()
        || params.min_revenue_yoy_percent.is_some()
        || params.min_revenue_yoy_streak.is_some()
        || params.min_revenue_yoy_3m_percent.is_some()
        || params.min_monthly_revenue.is_some()
        || params.min_eps.is_some()
        || params.min_roe_percent.is_some()
        || params.min_dividend_yield_percent.is_some()
        || params.min_f_score.is_some()
        || params.min_gross_margin_percent.is_some()
        || params.max_debt_ratio_percent.is_some()
        || params.positive_free_cash_flow == Some(true)
        || params.min_operating_margin_percent.is_some()
        || params.margins_yoy_up == Some(true)
        || params.min_rs_score.is_some()
        || params.min_foreign_streak.is_some()
        || params.min_trust_streak.is_some()
        || params.min_avg_trade_value.is_some();
    if !has_filter {
        return Err("至少需要一個篩選條件");
    }
    Ok(ValidatedScreeningParams {
        market_id,
        min_revenue_yoy_percent,
        min_revenue_yoy_3m_percent,
        min_monthly_revenue,
        min_eps,
        min_roe_percent,
        min_dividend_yield_percent,
        min_gross_margin_percent,
        max_debt_ratio_percent,
        min_operating_margin_percent,
        min_rs_score: params.min_rs_score.map(i16::from),
        min_foreign_streak: params.min_foreign_streak.map(i16::from),
        min_trust_streak: params.min_trust_streak.map(i16::from),
        min_avg_trade_value: params
            .min_avg_trade_value
            .map(|value| Decimal::from(value) * Decimal::from(1_000_000)),
        order_by,
        limit,
    })
}

/// 將有限範圍的 HTTP 浮點數轉成 PostgreSQL `NUMERIC` 綁定值。
///
/// JSON／query parser 可能接受 `NaN` 或無限大；先要求有限值，接著透過十進位
/// 字串建構 Decimal，確保 SQL 比較值與使用者輸入的十進位表示一致。
fn decimal_in_range(
    value: Option<f64>,
    minimum: f64,
    maximum: f64,
    error: &'static str,
) -> Result<Option<Decimal>, &'static str> {
    value
        .map(|number| {
            if !number.is_finite() || !(minimum..=maximum).contains(&number) {
                return Err(error);
            }
            Decimal::from_str(&number.to_string()).map_err(|_| error)
        })
        .transpose()
}

/// 將排序 enum 映射成二十八個固定 SQL 分支。
///
/// 指標欄位一律加 `NULLS LAST`，否則 PostgreSQL 的降冪預設會把過期或缺值的
/// NULL 排在最前面；同值再以股票代號升冪，讓分頁外的重複查詢仍穩定。
pub(super) fn screen_order_by(
    sort_by: &str,
    sort_order: &str,
) -> Result<&'static str, &'static str> {
    match (sort_by, sort_order) {
        ("stock_symbol", "asc") => Ok("ORDER BY stock_symbol ASC"),
        ("stock_symbol", "desc") => Ok("ORDER BY stock_symbol DESC"),
        ("revenue_yoy", "asc") => {
            Ok("ORDER BY revenue_yoy_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy", "desc") => {
            Ok("ORDER BY revenue_yoy_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_streak", "asc") => {
            Ok("ORDER BY revenue_yoy_streak ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_streak", "desc") => {
            Ok("ORDER BY revenue_yoy_streak DESC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_3m", "asc") => {
            Ok("ORDER BY revenue_yoy_3m_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_3m", "desc") => {
            Ok("ORDER BY revenue_yoy_3m_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("monthly_revenue", "asc") => {
            Ok("ORDER BY monthly_revenue ASC NULLS LAST, stock_symbol ASC")
        }
        ("monthly_revenue", "desc") => {
            Ok("ORDER BY monthly_revenue DESC NULLS LAST, stock_symbol ASC")
        }
        ("eps", "asc") => Ok("ORDER BY earnings_per_share ASC NULLS LAST, stock_symbol ASC"),
        ("eps", "desc") => Ok("ORDER BY earnings_per_share DESC NULLS LAST, stock_symbol ASC"),
        ("roe", "asc") => Ok("ORDER BY return_on_equity ASC NULLS LAST, stock_symbol ASC"),
        ("roe", "desc") => Ok("ORDER BY return_on_equity DESC NULLS LAST, stock_symbol ASC"),
        ("dividend_yield", "asc") => {
            Ok("ORDER BY dividend_yield_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("dividend_yield", "desc") => {
            Ok("ORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("valuation_percentage", "asc") => {
            Ok("ORDER BY valuation_percentage ASC NULLS LAST, stock_symbol ASC")
        }
        ("valuation_percentage", "desc") => {
            Ok("ORDER BY valuation_percentage DESC NULLS LAST, stock_symbol ASC")
        }
        ("f_score", "asc") => Ok("ORDER BY f_score ASC NULLS LAST, stock_symbol ASC"),
        ("f_score", "desc") => Ok("ORDER BY f_score DESC NULLS LAST, stock_symbol ASC"),
        ("gross_margin", "asc") => {
            Ok("ORDER BY gross_margin_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("gross_margin", "desc") => {
            Ok("ORDER BY gross_margin_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("debt_ratio", "asc") => Ok("ORDER BY debt_ratio_percent ASC NULLS LAST, stock_symbol ASC"),
        ("debt_ratio", "desc") => {
            Ok("ORDER BY debt_ratio_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("operating_margin", "asc") => {
            Ok("ORDER BY operating_margin_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("operating_margin", "desc") => {
            Ok("ORDER BY operating_margin_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("rs_score", "asc") => Ok("ORDER BY rs_score ASC NULLS LAST, stock_symbol ASC"),
        ("rs_score", "desc") => Ok("ORDER BY rs_score DESC NULLS LAST, stock_symbol ASC"),
        (_, "asc" | "desc") => Err("sort_by 不在允許範圍"),
        _ => Err("sort_order 必須為 asc 或 desc"),
    }
}

/// 測試 SQL 月份新鮮度公式的純 Rust 對照實作。
#[cfg(test)]
pub(super) fn revenue_month_is_fresh(query_date: NaiveDate, revenue_month: i64) -> bool {
    let query_ordinal = i64::from(query_date.year()) * 12 + i64::from(query_date.month());
    let revenue_ordinal = (revenue_month / 100) * 12 + revenue_month % 100;
    (0..=3).contains(&(query_ordinal - revenue_ordinal))
}

/// 測試 SQL 季度新鮮度公式的純 Rust 對照實作。
#[cfg(test)]
pub(super) fn financial_period_is_fresh(query_date: NaiveDate, year: i64, quarter: &str) -> bool {
    let Some(quarter) = quarter
        .strip_prefix('Q')
        .and_then(|raw| raw.parse::<i64>().ok())
    else {
        return false;
    };
    let query_ordinal = i64::from(query_date.year()) * 4 + i64::from(query_date.month0() / 3 + 1);
    let financial_ordinal = year * 4 + quarter;
    (0..=2).contains(&(query_ordinal - financial_ordinal))
}

/// 測試 SQL F 分數評分年度新鮮度公式的純 Rust 對照實作：落後查詢年 1–2 年。
#[cfg(test)]
pub(super) fn f_score_year_is_fresh(query_date: NaiveDate, f_score_year: i32) -> bool {
    (1..=2).contains(&(query_date.year() - f_score_year))
}

/// 測試 SQL 31 個日曆日視窗的純 Rust 對照實作。
#[cfg(test)]
pub(super) fn analytical_date_is_fresh(query_date: NaiveDate, source_date: NaiveDate) -> bool {
    source_date <= query_date && source_date >= query_date - chrono::Days::new(30)
}
