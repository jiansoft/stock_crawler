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
mod financial_health;
mod financial_reports;
mod industries;
mod industry_flow;
mod industry_strength;
mod institutional_flow;
mod institutional_streak;
mod margin_ranking;
mod market_calendar;
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
mod portfolio_history;
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

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use crate::domain::quote::technical::{SIGNAL_INDICATORS, SignalKind};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{ErrorBody, HealthResponse};

pub(super) use cagr::{__path_cagr_by_symbol, __path_cagr_ranking, cagr_by_symbol, cagr_ranking};
pub(super) use chip::{__path_stock_chip, stock_chip};
pub(super) use dividend_fill::{__path_dividend_fill, dividend_fill};
pub(super) use dividend_forecast::{__path_dividend_forecast, dividend_forecast};
pub(super) use etf::{
    __path_compare_etfs, __path_etf_detail, __path_list_etfs, compare_etfs, etf_detail, list_etfs,
};
pub(super) use financial_health::{__path_financial_health, financial_health};
pub(super) use financial_reports::{__path_financial_reports, financial_reports};
pub(super) use industries::{__path_industry_performance, industry_performance};
pub(super) use industry_flow::{__path_industry_flow, industry_flow};
pub(super) use industry_strength::{__path_industry_strength, industry_strength};
pub(super) use institutional_flow::{__path_institutional_flow, institutional_flow};
pub(super) use institutional_streak::{
    __path_institutional_streak_performance, institutional_streak_performance,
};
pub(super) use margin_ranking::{__path_margin_ranking, margin_ranking};
pub(super) use market_calendar::{__path_dividend_calendar, dividend_calendar};
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
pub(super) use portfolio_history::{__path_portfolio_history, portfolio_history};
pub(super) use relative_strength::{__path_relative_strength, relative_strength};
pub(super) use relative_strength_history::{
    __path_relative_strength_history, relative_strength_history,
};
pub use screen_backtest::BacktestStrategyGateway;
pub(crate) use screen_backtest::validate_strategy_query;
pub(super) use screen_backtest::{
    __path_screen_backtest, __path_screen_backtest_attribution, __path_screen_backtest_frontier,
    __path_screen_backtest_offsets, __path_screen_backtest_walk_forward, __path_strategy_presets,
    __path_strategy_track, screen_backtest, screen_backtest_attribution, screen_backtest_frontier,
    screen_backtest_offsets, screen_backtest_walk_forward, strategy_presets, strategy_track,
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

/// 產生不含內部實作細節的統一 JSON 錯誤回應。
pub(super) fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.to_owned(),
        }),
    )
        .into_response()
}

/// 將 PostgreSQL `NUMERIC` 安全轉換成 JSON number；超出 f64 範圍時保持 null。
fn decimal_to_f64(value: Option<Decimal>) -> Option<f64> {
    value.and_then(|number| number.to_string().parse().ok())
}

/// 將分析結果的 Decimal 轉為 JSON number，失敗時記錄固定欄位與股票代號。
///
/// API 仍依 §3.1 輸出 `null`；log 不包含 SQL、連線資訊或其他內部細節。
fn analytical_decimal_to_f64(
    value: Option<Decimal>,
    symbol: &str,
    field: &'static str,
) -> Option<f64> {
    value.and_then(|number| match number.to_string().parse() {
        Ok(converted) => Some(converted),
        Err(error) => {
            tracing::warn!(
                stock_symbol = symbol,
                field,
                ?error,
                "分析數值無法轉成 f64，API 將輸出 null"
            );
            None
        }
    })
}
/// 四捨五入到小數兩位（`f64::round` 為遠離零捨入）。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// 四捨五入到小數四位（比例、因子）。
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// 將資料庫 timestamp 轉為 UTC ISO 8601 格式。
fn timestamp(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|time| time.to_rfc3339())
}

/// 回傳不需認證的服務存活狀態。
#[utoipa::path(get, path = "/api/v1/healthz", tag = "data-api", responses((status = 200, body = HealthResponse)), security())]
pub(super) async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

/// 驗證並解析可選的日期區間。
fn parse_range(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(Option<NaiveDate>, Option<NaiveDate>), &'static str> {
    let parse = |value: Option<&str>| {
        value
            .map(|raw| {
                NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| "日期必須為 YYYY-MM-DD")
            })
            .transpose()
    };
    let (from, to) = (parse(from)?, parse(to)?);
    if from.zip(to).is_some_and(|(start, end)| start > end) {
        return Err("from 不可晚於 to");
    }
    Ok((from, to))
}

/// 解析分析 endpoint 共用的可選截止日。
///
/// `None` 代表由資料表自行取最新資料；有值但不是嚴格合法的
/// `YYYY-MM-DD` 時回 422 所使用的固定安全訊息。
fn parse_optional_date(value: Option<&str>) -> Result<Option<NaiveDate>, &'static str> {
    value
        .map(|raw| {
            // chrono 會接受未補零的月／日；契約要求固定十字元，先驗外觀，
            // 再以 chrono 驗證實際曆法（例如 2 月 30 日）。
            if raw.len() != 10 || raw.as_bytes()[4] != b'-' || raw.as_bytes()[7] != b'-' {
                return Err("日期必須為 YYYY-MM-DD");
            }
            NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| "日期必須為 YYYY-MM-DD")
        })
        .transpose()
}

/// 將市場名稱轉成 `daily_stock_price_stats` 的既有統計列 id（§3.6）。
fn market_id_for_stats(market: &str) -> Option<i32> {
    match market {
        "all" => Some(0),
        "twse" => Some(2),
        "tpex" => Some(4),
        _ => None,
    }
}

/// 將市場名稱轉成股票主檔篩選 id（§3.6）。
///
/// `0` 只是 handler 內部代表 `IN (2, 4)` 的哨兵；`stocks` 並不存在市場 0
/// 的合計列，SQL 不可直接用 `stock_exchange_market_id = 0`。
fn market_id_for_stocks(market: &str) -> Option<i32> {
    market_id_for_stats(market)
}

/// 依收盤價與三個加權估值分界產生固定分類（§4.4）。
///
/// 比較符號逐一對齊 `daily_stock_price_stats::upsert`：等於分界時歸入較低
/// 區間，確保個股 endpoint 與同日市場廣度統計能互相對帳。
fn valuation_band(
    closing_price: Decimal,
    cheap: Decimal,
    fair: Decimal,
    expensive: Decimal,
) -> &'static str {
    if closing_price <= cheap {
        "undervalued"
    } else if closing_price <= fair {
        "fair_valued"
    } else if closing_price <= expensive {
        "overvalued"
    } else {
        "highly_overvalued"
    }
}

/// 記錄內部資料庫錯誤並回傳安全的 500 訊息。
fn database_error(error: sqlx::Error) -> Response {
    tracing::error!(?error, "data API database query failed");
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "伺服器內部發生未預期錯誤",
    )
}

/// 驗證技術訊號的 `kind`／`indicator` 篩選值；不在固定清單時回 `Some(422)`。
fn signal_filter_error(kind: Option<&str>, indicator: Option<&str>) -> Option<Response> {
    if kind.is_some_and(|kind| SignalKind::parse(kind).is_none()) {
        return Some(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "kind 必須為 golden_cross、death_cross、cross_above、cross_below、overbought、oversold、new_high 或 new_low",
        ));
    }
    if indicator.is_some_and(|indicator| !SIGNAL_INDICATORS.contains(&indicator)) {
        return Some(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "indicator 必須為 ma5_ma20、ma20_ma60、kd、macd、close_ma20、close_ma60、close_ma240、rsi14、52_week、rs80 或 rs50",
        ));
    }
    None
}

/// 訊號來源在對外說明中的順序；未知來源排最後。
fn signal_indicator_order(indicator: &str) -> usize {
    SIGNAL_INDICATORS
        .iter()
        .position(|known| *known == indicator)
        .unwrap_or(usize::MAX)
}

/// 訊號種類在對外說明中的順序；未知種類排最後。
fn signal_kind_order(kind: &str) -> usize {
    SignalKind::parse(kind).map_or(usize::MAX, |kind| kind as usize)
}

/// 確認產業名稱存在（`None` 視為未篩選）；不存在回 `Some(422)`、查詢失敗回 `Some(500)`。
async fn industry_error(industry: Option<&str>) -> Option<Response> {
    let industry = industry?;
    let found: Result<Option<(i32,)>, _> =
        sqlx::query_as("SELECT stock_industry_id FROM stock_industry WHERE name = $1")
            .bind(industry)
            .fetch_optional(database::get_connection())
            .await;
    match found {
        Ok(Some(_)) => None,
        Ok(None) => Some(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "industry 必須為產業名稱，例如「半導體業」",
        )),
        Err(error) => Some(database_error(error)),
    }
}

/// 確認股票代號存在；不存在回 `Some(404)`、查詢失敗回 `Some(500)`，存在回 `None`。
///
/// 個股類 endpoint 共用此檢查，讓「未知代號 → 404」與「已知代號但指定
/// 範圍沒資料 → 200 空清單」兩種語意明確分開（§3.2），呼叫端（通常是
/// LLM）才能分辨「代號打錯」和「這支股票剛好沒這種資料」。
async fn ensure_stock_exists(symbol: &str) -> Option<Response> {
    let exists: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT stock_symbol FROM stocks WHERE stock_symbol = $1")
            .bind(symbol)
            .fetch_optional(database::get_connection())
            .await;
    match exists {
        Ok(Some(_)) => None,
        Ok(None) => Some(error_response(StatusCode::NOT_FOUND, "找不到股票代號")),
        Err(error) => Some(database_error(error)),
    }
}

/// 將 `YYYY-MM` 字串解析為資料庫使用的 `YYYYMM` 整數（例如 `2026-06` → `202606`）。
///
/// 格式或月份不合法時回錯誤訊息；年份限制四位數，與 `Revenue` 表的實際
/// 值域（P0-1：`201201`–`202606`）相容。
fn parse_month(value: &str) -> Result<i64, &'static str> {
    const ERROR: &str = "月份必須為 YYYY-MM";
    let (year, month) = value.split_once('-').ok_or(ERROR)?;
    if year.len() != 4 || month.len() != 2 {
        return Err(ERROR);
    }
    let year: i64 = year.parse().map_err(|_| ERROR)?;
    let month: i64 = month.parse().map_err(|_| ERROR)?;
    if !(1000..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(ERROR);
    }
    Ok(year * 100 + month)
}

/// 將資料庫的 `YYYYMM` 整數轉回 `YYYY-MM` 字串（`parse_month` 的反向操作）。
fn format_month(value: i64) -> String {
    format!("{:04}-{:02}", value / 100, value % 100)
}

/// 將資料庫的期間標記轉為 API 契約值（§3.5）：空字串（年度）→ `A`，其餘原樣。
fn quarter_to_api(quarter: &str) -> String {
    if quarter.is_empty() {
        "A".to_owned()
    } else {
        quarter.to_owned()
    }
}

/// 將資料庫的字串日期欄位清洗成 API 輸出值。
///
/// `dividend` 的日期欄位混雜 `-`、`尚未公布`、空字串，甚至殖利率字串
/// （P0-3 實測有 `1.39%` 這類髒資料）；只有能解析成合法 `YYYY-MM-DD`
/// 的值才輸出，其餘一律 `null`，不嘗試修補。
fn sanitize_date(value: &str) -> Option<String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|date| date.to_string())
}

/// 記錄倉儲層錯誤並回傳安全的 500 訊息。
///
/// 與 [`database_error`] 分開是因為倉儲回傳的是 `anyhow::Error`（帶 context
/// 鏈），對外仍只給固定訊息，不洩漏 SQL 或連線資訊。
fn repository_error(error: anyhow::Error) -> Response {
    tracing::error!(?error, "data API repository query failed");
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "伺服器內部發生未預期錯誤",
    )
}

#[cfg(test)]
mod tests;
