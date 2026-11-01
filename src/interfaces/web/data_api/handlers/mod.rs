//! Data API handlers 的共用工具與子模組彙整。
//!
//! 各組 endpoint 依路由分成 `stock_quotes`、`stock_fundamentals`、`financial_reports`、
//! `financial_health`、`dividend_fill`、`chip`、`market_stats`、`market_rankings`、
//! `market_movers`、`market_calendar`、`screening` 與 `cagr` 十二個子模組；此檔只保留跨組共用的錯誤轉換、數值與日期轉換
//! helper，並把 handler 重新導出，讓 `routes` 與 `openapi` 仍以
//! `handlers::<fn>` 引用。
//!
//! SQL 集中於各子模組，並一律使用 `$n` 參數化綁定；HTTP 錯誤不直接輸出
//! SQLx 錯誤，避免把資料庫主機、SQL 或堆疊資訊洩漏給呼叫端。

mod cagr;
mod chip;
mod dividend_fill;
mod dividend_forecast;
mod etf;
mod etf_tracking;
pub(super) use etf_tracking::{__path_etf_tracking, EtfTrackingResponse, etf_tracking};
mod etf_holding_history;
mod financial_health;
mod financial_reports;
mod holder_levels;
mod industries;
mod industry_flow;
mod industry_strength;
mod institutional_flow;
mod institutional_streak;
mod margin_ranking;
mod market_calendar;
mod market_dividend_fill;
mod market_forward_returns;
mod market_movers;
mod market_rankings;
mod market_regime;
mod market_signals;
mod market_stats;
mod market_valuation;
mod peers;
mod portfolio;
mod portfolio_chip;
mod portfolio_dividend_fill;
mod portfolio_etf_exposure;
mod portfolio_history;
mod portfolio_rebalance;
mod portfolio_target_revisions;
mod portfolio_targets;
mod relative_strength;
mod relative_strength_history;
mod screen_backtest;
mod screening;
mod signal_history;
mod stock_fundamentals;
mod stock_quotes;
mod technicals;
mod timeline;
mod valuation_band;

use axum::Json;

use crate::interfaces::web::data_api::dto::HealthResponse;

/// `min_major_change` 預設回看幾個集保資料週（選股與回測共用）。
pub(super) const DEFAULT_MAJOR_CHANGE_WEEKS: u8 = 4;

pub(super) use cagr::{__path_cagr_by_symbol, __path_cagr_ranking, cagr_by_symbol, cagr_ranking};
pub(super) use chip::{__path_stock_chip, stock_chip};
pub(super) use dividend_fill::{__path_dividend_fill, dividend_fill};
pub(super) use dividend_forecast::{__path_dividend_forecast, dividend_forecast};
pub(super) use etf::{
    __path_compare_etfs, __path_etf_detail, __path_list_etfs, compare_etfs, etf_detail, list_etfs,
};
pub(super) use etf_holding_history::{__path_etf_holding_history, etf_holding_history};
pub(super) use financial_health::{__path_financial_health, financial_health};
pub(super) use financial_reports::{__path_financial_reports, financial_reports};
pub(super) use holder_levels::{__path_holder_levels, holder_levels};
pub(super) use industries::{__path_industry_performance, industry_performance};
pub(super) use industry_flow::{__path_industry_flow, industry_flow};
pub(super) use industry_strength::{__path_industry_strength, industry_strength};
pub(super) use institutional_flow::{__path_institutional_flow, institutional_flow};
pub(super) use institutional_streak::{
    __path_institutional_streak_performance, institutional_streak_performance,
};
pub(super) use margin_ranking::{__path_margin_ranking, margin_ranking};
pub(super) use market_calendar::{__path_dividend_calendar, dividend_calendar};
pub(super) use market_dividend_fill::{
    __path_market_dividend_fill_stats, market_dividend_fill_stats,
};
pub(super) use market_forward_returns::{__path_market_forward_returns, market_forward_returns};
pub(super) use market_movers::{__path_market_movers, market_movers};
pub(super) use market_rankings::{
    __path_dividend_yield_ranking, __path_qfii_holding_ranking, dividend_yield_ranking,
    qfii_holding_ranking,
};
pub(super) use market_regime::{__path_market_regime, market_regime};
pub(super) use market_signals::{__path_market_signals, market_signals};
pub(super) use market_stats::{
    __path_market_breadth, __path_market_index_history, market_breadth, market_index_history,
};
pub(super) use market_valuation::{__path_market_valuation, market_valuation};
pub(super) use peers::{__path_stock_peers, stock_peers};
pub(super) use portfolio::{__path_portfolio_summary, portfolio_summary};
pub(super) use portfolio_chip::{__path_portfolio_chip, portfolio_chip};
pub(super) use portfolio_dividend_fill::{__path_portfolio_dividend_fill, portfolio_dividend_fill};
pub(super) use portfolio_etf_exposure::{__path_portfolio_etf_exposure, portfolio_etf_exposure};
pub(super) use portfolio_history::{__path_portfolio_history, portfolio_history};
pub(super) use portfolio_rebalance::{__path_portfolio_rebalance, portfolio_rebalance};
pub(super) use portfolio_target_revisions::{
    __path_list_portfolio_target_revisions, __path_restore_portfolio_target_revision,
    list_portfolio_target_revisions, restore_portfolio_target_revision,
};
pub(super) use portfolio_targets::{
    __path_get_portfolio_targets, __path_put_portfolio_targets, get_portfolio_targets,
    put_portfolio_targets,
};
pub(super) use relative_strength::{__path_relative_strength, relative_strength};
pub(super) use relative_strength_history::{
    __path_relative_strength_history, relative_strength_history,
};
pub use screen_backtest::BacktestStrategyGateway;
pub(crate) use screen_backtest::validate_strategy_query;
pub(super) use screen_backtest::{
    __path_screen_backtest, __path_screen_backtest_attribution, __path_screen_backtest_compare,
    __path_screen_backtest_frontier, __path_screen_backtest_offsets,
    __path_screen_backtest_walk_forward, __path_strategy_presets, __path_strategy_track,
    screen_backtest, screen_backtest_attribution, screen_backtest_compare,
    screen_backtest_frontier, screen_backtest_offsets, screen_backtest_walk_forward,
    strategy_presets, strategy_track,
};
pub(super) use screening::{__path_screen_stocks, screen_stocks};
pub(super) use signal_history::{
    __path_signal_performance, __path_stock_signals, signal_performance, stock_signals,
};
pub(super) use stock_fundamentals::{
    __path_dividend_history, __path_financial_statements, __path_monthly_revenues,
    __path_stock_valuation, dividend_history, financial_statements, monthly_revenues,
    stock_valuation,
};
pub(super) use stock_quotes::{
    __path_latest_quote, __path_price_history, __path_realtime_snapshot, __path_search_stocks,
    __path_stock_profile, latest_quote, price_history, realtime_snapshot, search_stocks,
    stock_profile,
};
pub(super) use technicals::{__path_stock_technicals, stock_technicals};
pub(super) use timeline::{__path_stock_timeline, stock_timeline};
pub(super) use valuation_band::{__path_stock_valuation_band, stock_valuation_band};

mod common;

pub(super) use self::common::error_response;
use self::common::{
    analytical_decimal_to_f64, database_error, decimal_to_f64, ensure_stock_exists, format_month,
    industry_error, market_id_for_stats, market_id_for_stocks, parse_month, parse_optional_date,
    parse_range, quarter_to_api, repository_error, round2, round4, sanitize_date,
    signal_filter_error, signal_indicator_order, signal_kind_order, timestamp, valuation_band,
};

/// 回傳不需認證的服務存活狀態。
#[utoipa::path(get, path = "/api/v1/healthz", tag = "data-api", responses((status = 200, body = HealthResponse)), security())]
pub(super) async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[cfg(test)]
mod tests;

mod strategy_edge;
pub(super) use strategy_edge::{__path_strategy_edge, strategy_edge};
