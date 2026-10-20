//! `/stocks/*` 的基本面歷史 handlers（月營收、財報、股利、估值）；對應的資料庫列型別與轉換在
//! [`rows`]。

mod rows;

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;

use self::rows::{DividendRow, RevenueRow, StatementRow, ValuationRow};
use super::{
    analytical_decimal_to_f64, database_error, ensure_stock_exists, error_response, format_month,
    parse_month, parse_optional_date, quarter_to_api, sanitize_date, timestamp, valuation_band,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    Dividend, DividendHistoryParams, DividendHistoryResponse, ErrorBody, FinancialStatement,
    FinancialStatementHistoryResponse, MonthlyRevenue, MonthlyRevenueResponse,
    RevenueHistoryParams, StatementHistoryParams, StockValuation, StockValuationResponse,
    ValuationParams,
};

/// 查詢單一股票的月營收歷史（§4.1）。
///
/// 流程：驗證參數 → 確認股票存在（未知代號回 404）→ 依月份區間查
/// `"Revenue"` → 轉成 API DTO 與 envelope。資料庫的月份是 `YYYYMM`
/// 整數（P0-1 已驗證全表合法），對外一律轉成 `YYYY-MM` 字串。
///
/// # Errors
///
/// 參數不合法回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗
/// 時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/monthly-revenues", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), RevenueHistoryParams), responses((status = 200, body = MonthlyRevenueResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn monthly_revenues(
    Path(symbol): Path<String>,
    Query(params): Query<RevenueHistoryParams>,
) -> Response {
    // 先驗證所有參數再碰資料庫：格式錯誤是呼叫端問題（422），
    // 不應消耗資料庫資源，也讓錯誤語意與資料存在與否無關。
    let from = match params.from.as_deref().map(parse_month).transpose() {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let to = match params.to.as_deref().map(parse_month).transpose() {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if from.zip(to).is_some_and(|(start, end)| start > end) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "from 不可晚於 to");
    }
    let limit = params.limit.unwrap_or(24);
    if !(1..=120).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 120");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    // `"SecurityCode"` 是 `Revenue_SecurityCode_Date-uidx` 的前導欄位，
    // P0-4 已驗證此查詢走索引反向掃描；`$2/$3` 為 NULL 時代表不限制區間。
    let rows: Result<Vec<RevenueRow>, _> = sqlx::query_as(r#"SELECT "Date" AS date, "Monthly" AS monthly, "LastMonth" AS last_month, "LastYearThisMonth" AS last_year_this_month, "MonthlyAccumulated" AS monthly_accumulated, "LastYearMonthlyAccumulated" AS last_year_monthly_accumulated, "ComparedWithLastMonth" AS compared_with_last_month, "ComparedWithLastYearSameMonth" AS compared_with_last_year_same_month, "AccumulatedComparedWithLastYear" AS accumulated_compared_with_last_year, avg_price, lowest_price, highest_price FROM "Revenue" WHERE "SecurityCode" = $1 AND ($2::bigint IS NULL OR "Date" >= $2) AND ($3::bigint IS NULL OR "Date" <= $3) ORDER BY "Date" DESC LIMIT $4"#).bind(&symbol).bind(from).bind(to).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => {
            // 清單固定新到舊（§3.1），因此第一筆就是最新一期；
            // 空清單時 `data_as_of` 維持 null，不揣測日期。
            let data_as_of = rows.first().map(|row| format_month(row.date));
            let revenues = rows.into_iter().map(|row| row.into_dto(&symbol)).collect();
            Json(MonthlyRevenueResponse {
                stock_symbol: symbol,
                data_as_of,
                revenues,
            })
            .into_response()
        }
        Err(error) => database_error(error),
    }
}

/// 查詢單一股票的季／年度財報歷史（§4.2）。
///
/// `period_type` 對映 §3.5：資料庫以空字串代表年度資料，因此
/// `annual` 過濾 `quarter = ''`、`quarterly` 過濾 `Q1`–`Q4`，輸出時
/// 空字串轉為 `A`。排序在 SQL 內以 `CASE quarter` 明確表達期間順序，
/// 不倚賴字典序（空字串的字典序在最前，與語意相反）。
///
/// # Errors
///
/// 參數不合法回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗
/// 時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/financial-statements", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), StatementHistoryParams), responses((status = 200, body = FinancialStatementHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn financial_statements(
    Path(symbol): Path<String>,
    Query(params): Query<StatementHistoryParams>,
) -> Response {
    let period_type = params.period_type.as_deref().unwrap_or("quarterly");
    if !["quarterly", "annual", "all"].contains(&period_type) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "period_type 必須為 quarterly、annual 或 all",
        );
    }
    let limit = params.limit.unwrap_or(12);
    if !(1..=40).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 40");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    // 期間過濾用單一 SQL 搭配 `$2` 判斷分支，避免動態拼接 SQL；
    // P0-4 已驗證 `(security_code, year, quarter)` 唯一索引可支撐此查詢。
    let rows: Result<Vec<StatementRow>, _> = sqlx::query_as(r#"SELECT "year", quarter, gross_profit, operating_profit_margin, "pre-tax_income" AS pre_tax_income, net_income, net_asset_value_per_share, sales_per_share, earnings_per_share, profit_before_tax, return_on_equity, return_on_assets, updated_time FROM financial_statement WHERE security_code = $1 AND ($2 = 'all' OR ($2 = 'annual' AND quarter = '') OR ($2 = 'quarterly' AND quarter IN ('Q1','Q2','Q3','Q4'))) ORDER BY "year" DESC, CASE quarter WHEN '' THEN 7 WHEN 'H2' THEN 6 WHEN 'H1' THEN 5 WHEN 'Q4' THEN 4 WHEN 'Q3' THEN 3 WHEN 'Q2' THEN 2 WHEN 'Q1' THEN 1 ELSE 0 END DESC LIMIT $3"#).bind(&symbol).bind(period_type).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => {
            let data_as_of = rows
                .first()
                .map(|row| format!("{}-{}", row.year, quarter_to_api(&row.quarter)));
            let statements = rows.into_iter().map(|row| row.into_dto(&symbol)).collect();
            Json(FinancialStatementHistoryResponse {
                stock_symbol: symbol,
                data_as_of,
                statements,
            })
            .into_response()
        }
        Err(error) => database_error(error),
    }
}

/// 查詢單一股票的股利發放歷史（§4.3）。
///
/// 年份篩選依「股利所屬年度」`year_of_dividend`（不是發放年度 `year`），
/// 避免兩種年度混淆。日期欄位在資料庫是字串且含 `-`、`尚未公布`、甚至
/// 殖利率字串等髒資料（P0-3 實測），只有合法 `YYYY-MM-DD` 才輸出。
///
/// # Errors
///
/// 參數不合法回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗
/// 時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/dividends", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), DividendHistoryParams), responses((status = 200, body = DividendHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_history(
    Path(symbol): Path<String>,
    Query(params): Query<DividendHistoryParams>,
) -> Response {
    // 年度上限取「目前年度加一」：股利政策常於年初公布下一年度配息，
    // 允許查到明年；再往後的年度必然無資料，直接視為參數錯誤。
    let max_year = chrono::Local::now().year() + 1;
    for year in [params.from_year, params.to_year].into_iter().flatten() {
        if !(1990..=max_year).contains(&year) {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "年度必須介於 1990 至目前年度加一",
            );
        }
    }
    if params
        .from_year
        .zip(params.to_year)
        .is_some_and(|(start, end)| start > end)
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "from_year 不可晚於 to_year",
        );
    }
    let limit = params.limit.unwrap_or(20);
    if !(1..=80).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 80");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    // 排序（§3.4）：股利所屬年度新到舊 → 同年度依 A、H2、H1、Q4…Q1 →
    // 最後以發放年度 DESC 穩定排序（同一期股利可能分年發放）。
    let rows: Result<Vec<DividendRow>, _> = sqlx::query_as(r#"SELECT "year", year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", earnings_cash_dividend, capital_reserve_cash_dividend, earnings_stock_dividend, capital_reserve_stock_dividend, payout_ratio_cash, payout_ratio_stock, payout_ratio, "ex-dividend_date1" AS ex_dividend_date1, "ex-dividend_date2" AS ex_dividend_date2, payable_date1, payable_date2, updated_time FROM dividend WHERE security_code = $1 AND ($2::int IS NULL OR year_of_dividend >= $2) AND ($3::int IS NULL OR year_of_dividend <= $3) ORDER BY year_of_dividend DESC, CASE quarter WHEN '' THEN 7 WHEN 'H2' THEN 6 WHEN 'H1' THEN 5 WHEN 'Q4' THEN 4 WHEN 'Q3' THEN 3 WHEN 'Q2' THEN 2 WHEN 'Q1' THEN 1 ELSE 0 END DESC, "year" DESC LIMIT $4"#).bind(&symbol).bind(params.from_year).bind(params.to_year).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => {
            let data_as_of = rows
                .first()
                .map(|row| format!("{}-{}", row.year_of_dividend, quarter_to_api(&row.quarter)));
            let dividends = rows.into_iter().map(|row| row.into_dto(&symbol)).collect();
            Json(DividendHistoryResponse {
                stock_symbol: symbol,
                data_as_of,
                dividends,
            })
            .into_response()
        }
        Err(error) => database_error(error),
    }
}

/// 查詢個股最近有效估值（§4.4）。
///
/// 指定日期時只在該日（含）往前 31 個日曆日內尋找，讓週末與連假能回到
/// 最近交易日，同時避免日期很舊時無限制掃描。股票存在但視窗內無估值時
/// 仍回 200，並以 `valuation: null` 表示資料缺值。
///
/// # Errors
///
/// 日期不合法回 422、股票不存在回 404、驗證失敗回 401；資料庫失敗回
/// 不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/valuation", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), ValuationParams), responses((status = 200, body = StockValuationResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_valuation(
    Path(symbol): Path<String>,
    Query(params): Query<ValuationParams>,
) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    // `(security_code, date)` 唯一索引支援代號等值與日期反向搜尋；指定截止日
    // 時的 `$2 - 30` 與截止日合計涵蓋 31 個日曆日。
    let row: Result<Option<ValuationRow>, _> = sqlx::query_as(r#"SELECT security_code, date, closing_price, percentage, year_count, cheap, fair, expensive, price_cheap, price_fair, price_expensive, dividend_cheap, dividend_fair, dividend_expensive, eps_cheap, eps_fair, eps_expensive, pbr_cheap, pbr_fair, pbr_expensive, per_cheap, per_fair, per_expensive FROM estimate WHERE security_code = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30)) ORDER BY date DESC LIMIT 1"#).bind(&symbol).bind(date).fetch_optional(database::get_connection()).await;
    match row {
        Ok(row) => {
            let valuation = row.map(Into::into);
            let data_as_of = valuation
                .as_ref()
                .map(|value: &StockValuation| value.date.clone());
            Json(StockValuationResponse {
                stock_symbol: symbol,
                data_as_of,
                valuation,
            })
            .into_response()
        }
        Err(error) => database_error(error),
    }
}
