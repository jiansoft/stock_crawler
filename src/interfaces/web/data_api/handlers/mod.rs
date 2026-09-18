//! Data API handlers 的共用工具與子模組彙整。
//!
//! 各組 endpoint 依路由分成 `stock_quotes`、`stock_fundamentals`、
//! `market_stats`、`market_rankings`、`market_calendar`、`screening` 與
//! `cagr` 七個子模組；此檔只保留跨組共用的錯誤轉換、數值與日期轉換
//! helper，並把 handler 重新導出，讓 `routes` 與 `openapi` 仍以
//! `handlers::<fn>` 引用。
//!
//! SQL 集中於各子模組，並一律使用 `$n` 參數化綁定；HTTP 錯誤不直接輸出
//! SQLx 錯誤，避免把資料庫主機、SQL 或堆疊資訊洩漏給呼叫端。

mod cagr;
mod market_calendar;
mod market_rankings;
mod market_stats;
mod screening;
mod stock_fundamentals;
mod stock_quotes;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use crate::infra::database;
use crate::interfaces::web::data_api::dto::{ErrorBody, HealthResponse};

pub(super) use cagr::{__path_cagr_by_symbol, __path_cagr_ranking, cagr_by_symbol, cagr_ranking};
pub(super) use market_calendar::{__path_dividend_calendar, dividend_calendar};
pub(super) use market_rankings::{
    __path_dividend_yield_ranking, __path_qfii_holding_ranking, dividend_yield_ranking,
    qfii_holding_ranking,
};
pub(super) use market_stats::{
    __path_market_breadth, __path_market_index_history, market_breadth, market_index_history,
};
pub(super) use screening::{__path_screen_stocks, screen_stocks};
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
mod tests {
    //! 純函式轉換邏輯的 deterministic tests（計畫 §9 Phase 1 要求）。
    //!
    //! 不需要資料庫：月份編碼、期間標記對映（§3.5）與股利日期清洗都是
    //! 純字串運算，行為必須與資料庫實際值域（P0-1～P0-3 實測）一一對應。

    use rust_decimal::Decimal;

    use super::{
        format_month, market_id_for_stats, market_id_for_stocks, parse_month, parse_optional_date,
        quarter_to_api, sanitize_date, valuation_band,
    };

    /// `YYYY-MM` ↔ `YYYYMM` 雙向轉換與各種非法輸入。
    #[test]
    fn month_conversion_roundtrip_and_validation() {
        assert_eq!(parse_month("2026-06"), Ok(202606));
        assert_eq!(format_month(202606), "2026-06");
        assert_eq!(parse_month("2012-01"), Ok(201201));
        // 月份超界、格式不符、缺零、混入其他字元都必須擋下。
        for invalid in [
            "2026-13", "2026-00", "2026-6", "202606", "26-06", "abcd-ef", "",
        ] {
            assert!(parse_month(invalid).is_err(), "{invalid:?} 應為非法月份");
        }
    }

    /// §3.5 期間標記對映：DB 空字串（年度）→ `A`，其餘原樣輸出。
    #[test]
    fn quarter_mapping_follows_section_3_5() {
        assert_eq!(quarter_to_api(""), "A");
        for passthrough in ["Q1", "Q2", "Q3", "Q4", "H1", "H2"] {
            assert_eq!(quarter_to_api(passthrough), passthrough);
        }
    }

    /// 股利日期清洗：合法日期原樣輸出；P0-3 實測的髒資料一律 `null`。
    #[test]
    fn dividend_date_sanitizing_drops_invalid_markers() {
        assert_eq!(sanitize_date("2026-07-15"), Some("2026-07-15".to_owned()));
        // `-`、`尚未公布`、空字串與殖利率字串都是資料庫實際存在的無效值。
        for invalid in ["-", "尚未公布", "", "1.39%", "2026/07/15", "2026-02-30"] {
            assert_eq!(sanitize_date(invalid), None, "{invalid:?} 應輸出 null");
        }
    }

    /// §4.4 四段估值分類的每個等號邊界必須與市場統計 SQL 完全一致。
    #[test]
    fn valuation_band_matches_market_breadth_boundaries() {
        let cheap = Decimal::new(100, 0);
        let fair = Decimal::new(200, 0);
        let expensive = Decimal::new(300, 0);
        assert_eq!(
            valuation_band(Decimal::new(100, 0), cheap, fair, expensive),
            "undervalued"
        );
        assert_eq!(
            valuation_band(Decimal::new(101, 0), cheap, fair, expensive),
            "fair_valued"
        );
        assert_eq!(
            valuation_band(Decimal::new(200, 0), cheap, fair, expensive),
            "fair_valued"
        );
        assert_eq!(
            valuation_band(Decimal::new(201, 0), cheap, fair, expensive),
            "overvalued"
        );
        assert_eq!(
            valuation_band(Decimal::new(300, 0), cheap, fair, expensive),
            "overvalued"
        );
        assert_eq!(
            valuation_band(Decimal::new(301, 0), cheap, fair, expensive),
            "highly_overvalued"
        );
    }

    /// §3.6 同一個 `market` 文字在統計表與股票表的 all 語意不同；helper
    /// 回傳的 0 對股票查詢只是 SQL 展開成 `IN (2,4)` 的內部哨兵。
    #[test]
    fn market_mapping_follows_section_3_6() {
        for mapper in [market_id_for_stats, market_id_for_stocks] {
            assert_eq!(mapper("all"), Some(0));
            assert_eq!(mapper("twse"), Some(2));
            assert_eq!(mapper("tpex"), Some(4));
            assert_eq!(mapper("emerging"), None);
        }
    }

    /// 分析 endpoints 共用日期解析必須拒絕不合法日期，避免 PostgreSQL 自行
    /// 寬鬆轉型造成不同 endpoint 行為不一致。
    #[test]
    fn analytics_date_validation_is_deterministic() {
        assert!(parse_optional_date(None).unwrap().is_none());
        assert_eq!(
            parse_optional_date(Some("2026-07-16"))
                .unwrap()
                .unwrap()
                .to_string(),
            "2026-07-16"
        );
        for invalid in ["2026-02-30", "2026/07/16", "2026-7-16", ""] {
            assert!(parse_optional_date(Some(invalid)).is_err());
        }
    }
}
