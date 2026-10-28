//! 條件選股的測試：[`validation`]（參數驗證、排序白名單與新鮮度公式的 deterministic tests），以及以真實
//! PostgreSQL 執行的整合測試——[`query_plans`]（執行計畫）、[`chip_liquidity`]（法人連買、成交金額）、
//! [`valuation_margins`]（估值上限、三率）、[`revenue_industry`]（營收新高、排除產業）；這裡放共用的參數建構函式。

mod chip_liquidity;
mod major;
mod query_plans;
mod revenue_industry;
mod validation;
mod valuation_margins;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::params::{
    analytical_date_is_fresh, f_score_year_is_fresh, financial_period_is_fresh,
    parse_exclude_industry_ids, revenue_month_is_fresh, screen_order_by, validate_screening_params,
};
use super::sql::SCREEN_STOCKS_SQL;
use super::{rs_columns, screen_at, streak_columns};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::StockScreeningParams;
use crate::interfaces::web::data_api::handlers::institutional_flow::StreakRow;
use crate::interfaces::web::data_api::handlers::relative_strength::{ScoredStock, UniverseRow};

/// 建立只覆寫測試重點欄位的 screen 預設參數。
pub(super) fn screening_params() -> StockScreeningParams {
    StockScreeningParams {
        market: None,
        industry_id: None,
        exclude_industry_ids: None,
        valuation_band: None,
        min_revenue_yoy_percent: None,
        min_revenue_yoy_streak: None,
        min_revenue_yoy_3m_percent: None,
        min_monthly_revenue: None,
        min_eps: None,
        min_roe_percent: None,
        min_dividend_yield_percent: None,
        min_f_score: None,
        min_gross_margin_percent: None,
        max_debt_ratio_percent: None,
        min_operating_margin_percent: None,
        margins_yoy_up: None,
        revenue_record: None,
        positive_free_cash_flow: None,
        min_rs_score: None,
        min_foreign_streak: None,
        min_trust_streak: None,
        min_avg_trade_value: None,
        max_pe: None,
        max_pb: None,
        min_major_change: None,
        major_change_weeks: None,
        sort_by: None,
        sort_order: None,
        limit: None,
    }
}
