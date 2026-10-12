//! `/stocks/screen` 條件選股 handler。
//!
//! 依職責拆成四個子模組：[`sql`] 參數化 SQL 主體、[`params`] 參數驗證與排序白名單、
//! [`row`] 資料庫列型別與 API 契約轉換，`tests` 為對應測試。

mod params;
mod row;
mod sql;
#[cfg(test)]
mod tests;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Local;

use self::{params::validate_screening_params, row::ScreenedStockRow, sql::SCREEN_STOCKS_SQL};
use super::{database_error, error_response};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, StockScreeningParams, StockScreeningResponse,
};

/// 以固定條件篩選上市櫃股票（§4.7）。
///
/// 每張分析表都先依股票代號取得該股票自己的最新一期，避免月營收分批公布
/// 時以全市場最大日期誤刪尚未公布的股票。接著以台北本地查詢日套用各來源
/// 的新鮮度上限；過期指標轉成 `null`，來源日期則保留供呼叫端稽核。
/// 財務體質（F 分數、毛利率、負債比、自由現金流）讀每日重算的 `financial_health`；
/// 營收動能（連續年增月數、近 3 月累計年增率）由最近 12 個月營收即時計算。
///
/// # Errors
///
/// 參數不在固定 enum／數值範圍、沒有任何有效篩選條件時回 422；資料庫查詢
/// 失敗時只記錄 server log，對外回不含 SQL 與連線資訊的安全 500 訊息。
#[utoipa::path(get, path = "/api/v1/stocks/screen", tag = "data-api", params(StockScreeningParams), responses((status = 200, body = StockScreeningResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_stocks(Query(params): Query<StockScreeningParams>) -> Response {
    let validated = match validate_screening_params(&params) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    // 查詢日由 handler 明確綁定，SQL 不直接讀 CURRENT_DATE；純函式測試可傳入
    // 任意日期驗證月份、季度與 31 天邊界，production 則採伺服器台北本地日。
    let query_date = Local::now().date_naive();
    // ORDER BY 字串只可能來自下方二十四個 &'static str 分支，絕不插入呼叫端
    // 原始文字；其他條件仍全部使用 bind parameter。
    let sql = format!("{SCREEN_STOCKS_SQL}\n{}\nLIMIT $16", validated.order_by);
    // SQLx 0.9 要求動態字串顯式稽核；此處唯一動態部分已由
    // `screen_order_by` 限制為二十四個靜態常數，因此可安全標記。
    let rows: Result<Vec<ScreenedStockRow>, _> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(validated.market_id)
        .bind(params.industry_id)
        .bind(query_date)
        .bind(validated.min_revenue_yoy_percent)
        .bind(validated.min_eps)
        .bind(validated.min_roe_percent)
        .bind(validated.min_dividend_yield_percent)
        .bind(params.valuation_band.as_deref())
        .bind(params.min_f_score.map(i16::from))
        .bind(validated.min_gross_margin_percent)
        .bind(validated.max_debt_ratio_percent)
        .bind(params.positive_free_cash_flow.unwrap_or(false))
        .bind(params.min_revenue_yoy_streak.map(i16::from))
        .bind(validated.min_revenue_yoy_3m_percent)
        .bind(validated.min_monthly_revenue)
        .bind(i64::from(validated.limit))
        .fetch_all(database::get_connection())
        .await;

    match rows {
        Ok(rows) => Json(StockScreeningResponse {
            // 每股各來源可能來自不同日期，因此不可偽造單一 data_as_of。
            data_as_of: None,
            stocks: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}
