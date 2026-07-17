//! Data API 的 Axum handlers 與唯讀 SQL 查詢。
//!
//! SQL 集中於此模組，並一律使用 `$n` 參數化綁定；HTTP 錯誤不直接輸出
//! SQLx 錯誤，避免把資料庫主機、SQL 或堆疊資訊洩漏給呼叫端。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::dto::{
    DailyQuote, Dividend, DividendHistoryParams, DividendHistoryResponse, DividendYieldRank,
    DividendYieldRankingParams, DividendYieldRankingResponse, ErrorBody, FinancialStatement,
    FinancialStatementHistoryResponse, HealthResponse, HistoricalQuote, HistoryParams,
    LatestQuoteResponse, MarketBreadth, MarketBreadthParams, MarketBreadthResponse, MonthlyRevenue,
    MonthlyRevenueResponse, PriceHistoryResponse, QuoteHistoryRecord, RealtimeSnapshotResponse,
    RevenueHistoryParams, ScreenedStock, SearchParams, SearchResponse, StatementHistoryParams,
    Stock, StockProfile, StockScreeningParams, StockScreeningResponse, StockValuation,
    StockValuationResponse, ValuationParams,
};
use crate::infra::{cache::SHARE, database};

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

/// 搜尋股票。
#[utoipa::path(get, path = "/api/v1/stocks/search", tag = "data-api", params(SearchParams), responses((status = 200, body = SearchResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn search_stocks(Query(params): Query<SearchParams>) -> Response {
    let query = params.query.trim();
    if !(1..=100).contains(&query.len()) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "query 長度必須介於 1 至 100 字元",
        );
    }
    let limit = params.limit.unwrap_or(10);
    if !(1..=50).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 50");
    }
    let wildcard = format!("%{query}%");
    let rows: Result<Vec<StockRow>, _> = sqlx::query_as(r#"SELECT stock_symbol, "SecurityCode" AS security_code, "Name" AS name, stock_exchange_market_id, stock_industry_id, "SuspendListing" AS suspend_listing FROM stocks WHERE stock_symbol = $1 OR "SecurityCode" ILIKE $2 OR "Name" ILIKE $2 ORDER BY CASE WHEN stock_symbol = $1 THEN 1 WHEN "SecurityCode" = $1 THEN 2 WHEN "SecurityCode" ILIKE $2 THEN 3 ELSE 4 END, stock_symbol ASC LIMIT $3"#).bind(query).bind(wildcard).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => Json(SearchResponse {
            stocks: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 查詢股票與其最新日報價。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/latest-quote", tag = "data-api", params(("symbol" = String, Path, description = "股票代號")), responses((status = 200, body = LatestQuoteResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn latest_quote(Path(symbol): Path<String>) -> Response {
    let row: Result<Option<LatestQuoteRow>, _> = sqlx::query_as(r#"SELECT s.stock_symbol, s."SecurityCode" AS security_code, s."Name" AS name, s.stock_exchange_market_id, s.stock_industry_id, s."SuspendListing" AS suspend_listing, q.date, q.opening_price, q.highest_price, q.lowest_price, q.closing_price, q.change, q.change_range, q.trading_volume, q.transaction, q.trade_value, q.moving_average_5, q.moving_average_10, q.moving_average_20, q.moving_average_60, q.moving_average_120, q.moving_average_240, q.price_earning_ratio, q.record_time, q.updated_time FROM stocks s LEFT JOIN last_daily_quotes q ON s.stock_symbol = q.stock_symbol WHERE s.stock_symbol = $1"#).bind(symbol).fetch_optional(database::get_connection()).await;
    match row {
        Ok(Some(row)) => Json(LatestQuoteResponse {
            stock: row.stock(),
            quote: row.quote(),
        })
        .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "找不到股票代號"),
        Err(error) => database_error(error),
    }
}

/// 查詢股票歷史日線資料；先確認股票存在，區分未知代號與空區間。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/price-history", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), HistoryParams), responses((status = 200, body = PriceHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn price_history(
    Path(symbol): Path<String>,
    Query(params): Query<HistoryParams>,
) -> Response {
    let (from, to) = match parse_range(params.from.as_deref(), params.to.as_deref()) {
        Ok(range) => range,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let limit = params.limit.unwrap_or(100);
    if !(1..=365).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 365");
    }
    // 主鍵存在性檢查讓「未知代號」明確是 404，而不是與空日期區間混為一談。
    let exists: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT stock_symbol FROM stocks WHERE stock_symbol = $1")
            .bind(&symbol)
            .fetch_optional(database::get_connection())
            .await;
    match exists {
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "找不到股票代號"),
        Err(error) => return database_error(error),
        Ok(Some(_)) => {}
    }
    let rows: Result<Vec<HistoricalQuoteRow>, _> = sqlx::query_as(r#"SELECT "Date" AS date, "OpeningPrice" AS opening_price, "HighestPrice" AS highest_price, "LowestPrice" AS lowest_price, "ClosingPrice" AS closing_price, "Change" AS change, "ChangeRange" AS change_range, "TradingVolume" AS trading_volume, "Transaction" AS transaction, "TradeValue" AS trade_value, "MovingAverage5" AS moving_average_5, "MovingAverage10" AS moving_average_10, "MovingAverage20" AS moving_average_20, "MovingAverage60" AS moving_average_60, "PriceEarningRatio" AS price_earning_ratio, "price-to-book_ratio" AS price_to_book_ratio, "RecordTime" AS record_time FROM "DailyQuotes" WHERE stock_symbol = $1 AND ($2::date IS NULL OR "Date" >= $2::date) AND ($3::date IS NULL OR "Date" <= $3::date) ORDER BY "Date" DESC LIMIT $4"#).bind(symbol).bind(from).bind(to).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => Json(PriceHistoryResponse {
            quotes: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 查詢股票完整基本面、最新日報價與歷史高低點。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/profile", tag = "data-api", params(("symbol" = String, Path, description = "股票代號")), responses((status = 200, body = StockProfile), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn stock_profile(Path(symbol): Path<String>) -> Response {
    let row: Result<Option<ProfileRow>, _> = sqlx::query_as(r#"SELECT s.stock_symbol, s."SecurityCode" AS security_code, s."Name" AS name, s.stock_exchange_market_id, s.stock_industry_id, s."SuspendListing" AS suspend_listing, s.last_one_eps, s.last_four_eps, s.net_asset_value_per_share, s.return_on_equity, s.weight, s.issued_share, q.date, q.opening_price, q.highest_price, q.lowest_price, q.closing_price, q.change, q.change_range, q.trading_volume, q.transaction, q.trade_value, q.moving_average_5, q.moving_average_10, q.moving_average_20, q.moving_average_60, q.moving_average_120, q.moving_average_240, q.price_earning_ratio, q.record_time, q.updated_time, h.maximum_price, h.maximum_price_date_on, h.minimum_price, h.minimum_price_date_on, h."maximum_price-to-book_ratio" AS maximum_price_to_book_ratio, h."maximum_price-to-book_ratio_date_on" AS maximum_price_to_book_ratio_date_on, h."minimum_price-to-book_ratio" AS minimum_price_to_book_ratio, h."minimum_price-to-book_ratio_date_on" AS minimum_price_to_book_ratio_date_on FROM stocks s LEFT JOIN last_daily_quotes q ON s.stock_symbol = q.stock_symbol LEFT JOIN quote_history_record h ON s."SecurityCode" = h.security_code WHERE s.stock_symbol = $1"#).bind(symbol).fetch_optional(database::get_connection()).await;
    match row {
        Ok(Some(row)) => Json::<StockProfile>(row.into()).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "找不到股票代號"),
        Err(error) => database_error(error),
    }
}

/// 查詢第三方採集的近即時報價快照，不宣稱為交易所保證即時行情。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/realtime-snapshot", tag = "data-api", params(("symbol" = String, Path, description = "股票代號")), responses((status = 200, body = RealtimeSnapshotResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn realtime_snapshot(Path(symbol): Path<String>) -> Response {
    if SHARE.stock_snapshots_are_empty() {
        return error_response(StatusCode::NOT_FOUND, "目前非交易時段,無即時報價快照");
    }
    let Some(snapshot) = SHARE.get_stock_snapshot(&symbol) else {
        return error_response(StatusCode::NOT_FOUND, "查無此股票的即時報價快照");
    };
    let convert = |value: Decimal| value.to_string().parse::<f64>().ok();
    Json(RealtimeSnapshotResponse {
        stock_symbol: snapshot.symbol,
        name: snapshot.name,
        price: convert(snapshot.price),
        change: convert(snapshot.change),
        change_range: convert(snapshot.change_range),
        open: convert(snapshot.open),
        high: convert(snapshot.high),
        low: convert(snapshot.low),
        last_close: convert(snapshot.last_close),
        volume_lots: convert(snapshot.volume),
        source_site: snapshot.source_site,
        updated_at: snapshot.updated_at.to_rfc3339(),
    })
    .into_response()
}

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
pub(super) async fn monthly_revenues(
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
pub(super) async fn financial_statements(
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
pub(super) async fn dividend_history(
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
pub(super) async fn stock_valuation(
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

/// 查詢指定市場最近數個交易日的市場廣度（§4.5）。
///
/// `days` 計算的是資料表中實際存在的交易日列，不會為休市日補零；回應的
/// `breadth` 一律複製 `history[0]`，使單日與序列查詢維持相同 JSON 形狀。
///
/// # Errors
///
/// 市場、日期或 days 不合法回 422，查無任何統計回 404，驗證失敗回 401，
/// 資料庫失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/breadth", tag = "data-api", params(MarketBreadthParams), responses((status = 200, body = MarketBreadthResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn market_breadth(Query(params): Query<MarketBreadthParams>) -> Response {
    let market = params.market.as_deref().unwrap_or("all");
    let market_id = match market_id_for_stats(market) {
        Some(value) => value,
        None => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "market 必須為 all、twse 或 tpex",
            );
        }
    };
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(1);
    if !(1..=60).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }
    // 統計表主鍵／唯一鍵以市場與日期定位。指定日期時只允許最新一筆落在
    // 31 天回溯窗內；一旦找到終點，再向前取 N 個「有資料的交易日」。
    let rows: Result<Vec<MarketBreadthRow>, _> = sqlx::query_as(r#"WITH endpoint AS (SELECT MAX(date) AS date FROM daily_stock_price_stats WHERE stock_exchange_market_id = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30))) SELECT s.date, s.undervalued, s.fair_valued, s.overvalued, s.highly_overvalued, s.below_5_day_moving_average, s.above_5_day_moving_average, s.below_20_day_moving_average, s.above_20_day_moving_average, s.below_60_day_moving_average, s.above_60_day_moving_average, s.below_120_day_moving_average, s.above_120_day_moving_average, s.below_240_day_moving_average, s.above_240_day_moving_average, s.stocks_up, s.stocks_down, s.stocks_unchanged, s.updated_at FROM daily_stock_price_stats s CROSS JOIN endpoint e WHERE s.stock_exchange_market_id = $1 AND s.date <= e.date ORDER BY s.date DESC LIMIT $3"#).bind(market_id).bind(date).bind(i64::from(days)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) if rows.is_empty() => error_response(StatusCode::NOT_FOUND, "查無市場廣度資料"),
        Ok(rows) => {
            let history: Vec<MarketBreadth> =
                rows.into_iter().map(|row| row.into_dto(market)).collect();
            Json(build_market_breadth_response(history).expect("已排除空清單")).into_response()
        }
        Err(error) => database_error(error),
    }
}

/// 查詢單一有效交易日的殖利率排行（§4.6）。
///
/// 先從 `yield_rank` 決定 31 天視窗內唯一資料日，再 JOIN 原始報價與股利列，
/// 因此同一份排行不會混入不同日期。市場條件只接受上市（2）與上櫃（4），
/// `all` 也刻意排除公開發行與興櫃，因其收盤價資料不足以形成可靠排名。
///
/// # Errors
///
/// 參數不合法回 422，回溯窗內沒有排行日期回 404，驗證失敗回 401，資料庫
/// 失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/dividend-yield-ranking", tag = "data-api", params(DividendYieldRankingParams), responses((status = 200, body = DividendYieldRankingResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn dividend_yield_ranking(
    Query(params): Query<DividendYieldRankingParams>,
) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let market = params.market.as_deref().unwrap_or("all");
    let market_id = match market_id_for_stocks(market) {
        Some(value) => value,
        None => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "market 必須為 all、twse 或 tpex",
            );
        }
    };
    if params.industry_id.is_some_and(|value| value <= 0) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "industry_id 必須為正整數");
    }
    let limit = params.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 50");
    }
    // 先單獨取排行日期，才能區分「整張表／回溯窗沒有日期」（404）與
    // 「日期存在但市場或產業篩選後無股票」（200 空陣列）。
    let data_date: Result<Option<(Option<NaiveDate>,)>, _> = sqlx::query_as(r#"SELECT MAX(date) FROM yield_rank WHERE $1::date IS NULL OR (date <= $1 AND date >= $1 - 30)"#).bind(date).fetch_optional(database::get_connection()).await;
    let data_date = match data_date {
        Ok(Some((Some(value),))) => value,
        Ok(Some((None,))) | Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "查無殖利率排行資料");
        }
        Err(error) => return database_error(error),
    };
    // `$2 = 0` 是 API 內部的 all 哨兵，只展開為固定 `IN (2,4)`；所有排序
    // 欄位均寫死，呼叫端無法提供 SQL 片段或欄位名稱。
    let rows: Result<Vec<YieldRankRow>, _> = sqlx::query_as(r#"SELECT y.security_code AS stock_symbol, s."Name" AS name, s.stock_exchange_market_id AS market_id, s.stock_industry_id AS industry_id, y.date, q."ClosingPrice" AS closing_price, d."sum" AS dividend, y.yield AS dividend_yield_percent FROM yield_rank y JOIN stocks s ON s.stock_symbol = y.security_code JOIN "DailyQuotes" q ON q."Serial" = y.daily_quotes_serial JOIN dividend d ON d.serial = y.dividend_serial WHERE y.date = $1 AND (($2 = 0 AND s.stock_exchange_market_id IN (2, 4)) OR s.stock_exchange_market_id = $2) AND ($3::int IS NULL OR s.stock_industry_id = $3) ORDER BY y.yield DESC, y.security_code ASC LIMIT $4"#).bind(data_date).bind(market_id).bind(params.industry_id).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => Json(DividendYieldRankingResponse {
            data_as_of: data_date.to_string(),
            stocks: rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| row.into_dto(index as u32 + 1))
                .collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 以固定條件篩選上市櫃股票（§4.7）。
///
/// 每張分析表都先依股票代號取得該股票自己的最新一期，避免月營收分批公布
/// 時以全市場最大日期誤刪尚未公布的股票。接著以台北本地查詢日套用各來源
/// 的新鮮度上限；過期指標轉成 `null`，來源日期則保留供呼叫端稽核。
///
/// # Errors
///
/// 參數不在固定 enum／數值範圍、沒有任何有效篩選條件時回 422；資料庫查詢
/// 失敗時只記錄 server log，對外回不含 SQL 與連線資訊的安全 500 訊息。
#[utoipa::path(get, path = "/api/v1/stocks/screen", tag = "data-api", params(StockScreeningParams), responses((status = 200, body = StockScreeningResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(super) async fn screen_stocks(Query(params): Query<StockScreeningParams>) -> Response {
    let validated = match validate_screening_params(&params) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    // 查詢日由 handler 明確綁定，SQL 不直接讀 CURRENT_DATE；純函式測試可傳入
    // 任意日期驗證月份、季度與 31 天邊界，production 則採伺服器台北本地日。
    let query_date = Local::now().date_naive();
    // ORDER BY 字串只可能來自下方十二個 &'static str 分支，絕不插入呼叫端
    // 原始文字；其他條件仍全部使用 bind parameter。
    let sql = format!("{SCREEN_STOCKS_SQL}\n{}\nLIMIT $9", validated.order_by);
    // SQLx 0.9 要求動態字串顯式稽核；此處唯一動態部分已由
    // `screen_order_by` 限制為十二個靜態常數，因此可安全標記。
    let rows: Result<Vec<ScreenedStockRow>, _> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(validated.market_id)
        .bind(params.industry_id)
        .bind(query_date)
        .bind(validated.min_revenue_yoy_percent)
        .bind(validated.min_eps)
        .bind(validated.min_roe_percent)
        .bind(validated.min_dividend_yield_percent)
        .bind(params.valuation_band.as_deref())
        .bind(i64::from(validated.limit))
        .fetch_all(database::get_connection())
        .await;

    match rows {
        Ok(rows) => Json(StockScreeningResponse {
            // 每股四種資料可能來自不同日期，因此不可偽造單一 data_as_of。
            data_as_of: None,
            stocks: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 條件選股的參數化 SQL 主體。
///
/// `LATERAL ... LIMIT 1` 會使用各表以股票代號為前導欄位的索引，找出「每股
/// 各自最新」資料。`normalized` 僅將過期指標轉成 SQL NULL；最外層條件利用
/// NULL 比較不成立的語意，自然排除以該過期指標篩選的股票。
const SCREEN_STOCKS_SQL: &str = r#"
WITH latest AS (
    SELECT
        s.stock_symbol,
        s."Name" AS name,
        s.stock_exchange_market_id AS market_id,
        s.stock_industry_id AS industry_id,
        r."Date" AS revenue_raw_month,
        r."ComparedWithLastYearSameMonth" AS revenue_raw_yoy,
        f.year AS financial_year,
        f.quarter AS financial_quarter,
        f.earnings_per_share AS eps_raw,
        f.return_on_equity AS roe_raw,
        e.date AS valuation_raw_date,
        e.percentage AS valuation_raw_percentage,
        e.closing_price,
        e.cheap,
        e.fair,
        e.expensive,
        y.date AS yield_raw_date,
        y.yield AS yield_raw
    FROM stocks s
    LEFT JOIN LATERAL (
        SELECT "Date", "ComparedWithLastYearSameMonth"
        FROM "Revenue"
        WHERE "SecurityCode" = s.stock_symbol
        ORDER BY "Date" DESC
        LIMIT 1
    ) r ON TRUE
    LEFT JOIN LATERAL (
        SELECT year, quarter, earnings_per_share, return_on_equity
        FROM financial_statement
        WHERE security_code = s.stock_symbol
          AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
        ORDER BY year DESC,
            CASE quarter
                WHEN 'Q4' THEN 4 WHEN 'Q3' THEN 3
                WHEN 'Q2' THEN 2 WHEN 'Q1' THEN 1
            END DESC
        LIMIT 1
    ) f ON TRUE
    LEFT JOIN LATERAL (
        SELECT date, percentage, closing_price, cheap, fair, expensive
        FROM estimate
        WHERE security_code = s.stock_symbol
        ORDER BY date DESC
        LIMIT 1
    ) e ON TRUE
    LEFT JOIN LATERAL (
        SELECT date, yield
        FROM yield_rank
        WHERE security_code = s.stock_symbol
        ORDER BY date DESC
        LIMIT 1
    ) y ON TRUE
    WHERE (($1::int = 0 AND s.stock_exchange_market_id IN (2, 4))
        OR s.stock_exchange_market_id = $1)
      AND ($2::int IS NULL OR s.stock_industry_id = $2)
), normalized AS (
    SELECT
        latest.*,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_raw_yoy END AS revenue_yoy_percent,
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN eps_raw END AS earnings_per_share,
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN roe_raw END AS return_on_equity,
        CASE WHEN valuation_raw_date BETWEEN $3::date - 30 AND $3::date
          THEN valuation_raw_percentage END AS valuation_percentage,
        CASE WHEN valuation_raw_date BETWEEN $3::date - 30 AND $3::date THEN
            CASE
                WHEN closing_price <= cheap THEN 'undervalued'
                WHEN closing_price <= fair THEN 'fair_valued'
                WHEN closing_price <= expensive THEN 'overvalued'
                ELSE 'highly_overvalued'
            END
        END AS valuation_band,
        CASE WHEN yield_raw_date BETWEEN $3::date - 30 AND $3::date
          THEN yield_raw END AS dividend_yield_percent
    FROM latest
)
SELECT
    stock_symbol, name, market_id, industry_id,
    revenue_yoy_percent, earnings_per_share, return_on_equity,
    dividend_yield_percent, valuation_band, valuation_percentage,
    revenue_raw_month AS revenue_month,
    financial_year, financial_quarter,
    valuation_raw_date AS valuation_date,
    yield_raw_date AS yield_date
FROM normalized
WHERE ($4::numeric IS NULL OR revenue_yoy_percent >= $4)
  AND ($5::numeric IS NULL OR earnings_per_share >= $5)
  AND ($6::numeric IS NULL OR return_on_equity >= $6)
  AND ($7::numeric IS NULL OR dividend_yield_percent >= $7)
  AND ($8::text IS NULL OR valuation_band = $8)
"#;

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

/// 驗證後可安全綁定條件選股 SQL 的內部參數。
///
/// 數值門檻預先轉為 `Decimal`，使 PostgreSQL 能以原生 `NUMERIC` 比較而不會
/// 因浮點參數產生隱含型別轉換；排序則只保留程式內建的靜態 SQL 片段。
struct ValidatedScreeningParams {
    /// 股票表市場 id；0 只代表 SQL 內固定展開的上市加上櫃。
    market_id: i32,
    /// 最低營收年增率。
    min_revenue_yoy_percent: Option<Decimal>,
    /// 最低每股盈餘。
    min_eps: Option<Decimal>,
    /// 最低股東權益報酬率。
    min_roe_percent: Option<Decimal>,
    /// 最低殖利率。
    min_dividend_yield_percent: Option<Decimal>,
    /// 十二個固定排序分支之一。
    order_by: &'static str,
    /// 查詢筆數上限。
    limit: u8,
}

/// 驗證條件選股參數並轉換成資料庫可直接綁定的型別。
///
/// `market=twse/tpex` 本身就是有效篩選；`market=all` 不縮小上市櫃集合，
/// 因此必須再提供產業、估值或至少一個數值門檻，避免 endpoint 被當成無條件
/// 的全市場匯出介面。
fn validate_screening_params(
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
        || params.min_eps.is_some()
        || params.min_roe_percent.is_some()
        || params.min_dividend_yield_percent.is_some();
    if !has_filter {
        return Err("至少需要一個篩選條件");
    }
    Ok(ValidatedScreeningParams {
        market_id,
        min_revenue_yoy_percent,
        min_eps,
        min_roe_percent,
        min_dividend_yield_percent,
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

/// 將排序 enum 映射成十二個固定 SQL 分支。
///
/// 指標欄位一律加 `NULLS LAST`，否則 PostgreSQL 的降冪預設會把過期或缺值的
/// NULL 排在最前面；同值再以股票代號升冪，讓分頁外的重複查詢仍穩定。
fn screen_order_by(sort_by: &str, sort_order: &str) -> Result<&'static str, &'static str> {
    match (sort_by, sort_order) {
        ("stock_symbol", "asc") => Ok("ORDER BY stock_symbol ASC"),
        ("stock_symbol", "desc") => Ok("ORDER BY stock_symbol DESC"),
        ("revenue_yoy", "asc") => {
            Ok("ORDER BY revenue_yoy_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy", "desc") => {
            Ok("ORDER BY revenue_yoy_percent DESC NULLS LAST, stock_symbol ASC")
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
        (_, "asc" | "desc") => Err("sort_by 不在允許範圍"),
        _ => Err("sort_order 必須為 asc 或 desc"),
    }
}

/// 測試 SQL 月份新鮮度公式的純 Rust 對照實作。
#[cfg(test)]
fn revenue_month_is_fresh(query_date: NaiveDate, revenue_month: i64) -> bool {
    let query_ordinal = i64::from(query_date.year()) * 12 + i64::from(query_date.month());
    let revenue_ordinal = (revenue_month / 100) * 12 + revenue_month % 100;
    (0..=3).contains(&(query_ordinal - revenue_ordinal))
}

/// 測試 SQL 季度新鮮度公式的純 Rust 對照實作。
#[cfg(test)]
fn financial_period_is_fresh(query_date: NaiveDate, year: i64, quarter: &str) -> bool {
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

/// 測試 SQL 31 個日曆日視窗的純 Rust 對照實作。
#[cfg(test)]
fn analytical_date_is_fresh(query_date: NaiveDate, source_date: NaiveDate) -> bool {
    source_date <= query_date && source_date >= query_date - chrono::Days::new(30)
}

/// 由新到舊的廣度序列建立固定 envelope；空序列代表沒有可用資料。
///
/// `breadth` 直接複製第一筆，從資料流保證它等於 `history[0]`，並讓單日與
/// 多日查詢維持相同 response 形狀。
fn build_market_breadth_response(history: Vec<MarketBreadth>) -> Option<MarketBreadthResponse> {
    let breadth = history.first()?.clone();
    Some(MarketBreadthResponse {
        data_as_of: breadth.date.clone(),
        breadth,
        history,
    })
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

/// 對應 `stocks` 搜尋列。
#[derive(sqlx::FromRow)]
struct StockRow {
    stock_symbol: String,
    security_code: String,
    name: String,
    stock_exchange_market_id: i32,
    stock_industry_id: i32,
    suspend_listing: bool,
}
impl From<StockRow> for Stock {
    fn from(row: StockRow) -> Self {
        Self {
            stock_symbol: row.stock_symbol,
            security_code: row.security_code,
            name: row.name,
            stock_exchange_market_id: row.stock_exchange_market_id,
            stock_industry_id: row.stock_industry_id,
            suspend_listing: row.suspend_listing,
        }
    }
}
/// 對應股票與可選最新日報價的 LEFT JOIN 列。
#[derive(sqlx::FromRow)]
struct LatestQuoteRow {
    stock_symbol: String,
    security_code: String,
    name: String,
    stock_exchange_market_id: i32,
    stock_industry_id: i32,
    suspend_listing: bool,
    date: Option<NaiveDate>,
    opening_price: Option<Decimal>,
    highest_price: Option<Decimal>,
    lowest_price: Option<Decimal>,
    closing_price: Option<Decimal>,
    change: Option<Decimal>,
    change_range: Option<Decimal>,
    trading_volume: Option<Decimal>,
    transaction: Option<Decimal>,
    trade_value: Option<Decimal>,
    moving_average_5: Option<Decimal>,
    moving_average_10: Option<Decimal>,
    moving_average_20: Option<Decimal>,
    moving_average_60: Option<Decimal>,
    moving_average_120: Option<Decimal>,
    moving_average_240: Option<Decimal>,
    price_earning_ratio: Option<Decimal>,
    record_time: Option<DateTime<Utc>>,
    updated_time: Option<DateTime<Utc>>,
}
impl LatestQuoteRow {
    /// 複製 LEFT JOIN 左側一定存在的股票主檔欄位。
    fn stock(&self) -> Stock {
        Stock {
            stock_symbol: self.stock_symbol.clone(),
            security_code: self.security_code.clone(),
            name: self.name.clone(),
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_industry_id: self.stock_industry_id,
            suspend_listing: self.suspend_listing,
        }
    }
    /// 若 JOIN 到日報價則轉為 API DTO，否則維持 JSON null 語意。
    fn quote(self) -> Option<DailyQuote> {
        self.date.map(|date| DailyQuote {
            date: date.to_string(),
            opening_price: decimal_to_f64(self.opening_price),
            highest_price: decimal_to_f64(self.highest_price),
            lowest_price: decimal_to_f64(self.lowest_price),
            closing_price: decimal_to_f64(self.closing_price),
            change: decimal_to_f64(self.change),
            change_range: decimal_to_f64(self.change_range),
            trading_volume: decimal_to_f64(self.trading_volume),
            transaction: decimal_to_f64(self.transaction),
            trade_value: decimal_to_f64(self.trade_value),
            moving_average_5: decimal_to_f64(self.moving_average_5),
            moving_average_10: decimal_to_f64(self.moving_average_10),
            moving_average_20: decimal_to_f64(self.moving_average_20),
            moving_average_60: decimal_to_f64(self.moving_average_60),
            moving_average_120: decimal_to_f64(self.moving_average_120),
            moving_average_240: decimal_to_f64(self.moving_average_240),
            price_earning_ratio: decimal_to_f64(self.price_earning_ratio),
            record_time: timestamp(self.record_time),
            updated_time: timestamp(self.updated_time),
        })
    }
}
/// 對應歷史日線列。
#[derive(sqlx::FromRow)]
struct HistoricalQuoteRow {
    date: NaiveDate,
    opening_price: Option<Decimal>,
    highest_price: Option<Decimal>,
    lowest_price: Option<Decimal>,
    closing_price: Option<Decimal>,
    change: Option<Decimal>,
    change_range: Option<Decimal>,
    trading_volume: Option<Decimal>,
    transaction: Option<Decimal>,
    trade_value: Option<Decimal>,
    moving_average_5: Option<Decimal>,
    moving_average_10: Option<Decimal>,
    moving_average_20: Option<Decimal>,
    moving_average_60: Option<Decimal>,
    price_earning_ratio: Option<Decimal>,
    price_to_book_ratio: Option<Decimal>,
    record_time: Option<DateTime<Utc>>,
}
impl From<HistoricalQuoteRow> for HistoricalQuote {
    fn from(row: HistoricalQuoteRow) -> Self {
        Self {
            date: row.date.to_string(),
            opening_price: decimal_to_f64(row.opening_price),
            highest_price: decimal_to_f64(row.highest_price),
            lowest_price: decimal_to_f64(row.lowest_price),
            closing_price: decimal_to_f64(row.closing_price),
            change: decimal_to_f64(row.change),
            change_range: decimal_to_f64(row.change_range),
            trading_volume: decimal_to_f64(row.trading_volume),
            transaction: decimal_to_f64(row.transaction),
            trade_value: decimal_to_f64(row.trade_value),
            moving_average_5: decimal_to_f64(row.moving_average_5),
            moving_average_10: decimal_to_f64(row.moving_average_10),
            moving_average_20: decimal_to_f64(row.moving_average_20),
            moving_average_60: decimal_to_f64(row.moving_average_60),
            price_earning_ratio: decimal_to_f64(row.price_earning_ratio),
            price_to_book_ratio: decimal_to_f64(row.price_to_book_ratio),
            record_time: timestamp(row.record_time),
        }
    }
}
/// 對應 profile 三表 LEFT JOIN 列。
#[derive(sqlx::FromRow)]
struct ProfileRow {
    #[sqlx(flatten)]
    latest: LatestQuoteRow,
    last_one_eps: Option<Decimal>,
    last_four_eps: Option<Decimal>,
    net_asset_value_per_share: Option<Decimal>,
    return_on_equity: Option<Decimal>,
    weight: Option<Decimal>,
    issued_share: Option<Decimal>,
    maximum_price: Option<Decimal>,
    maximum_price_date_on: Option<NaiveDate>,
    minimum_price: Option<Decimal>,
    minimum_price_date_on: Option<NaiveDate>,
    maximum_price_to_book_ratio: Option<Decimal>,
    maximum_price_to_book_ratio_date_on: Option<NaiveDate>,
    minimum_price_to_book_ratio: Option<Decimal>,
    minimum_price_to_book_ratio_date_on: Option<NaiveDate>,
}
/// 對應 `"Revenue"` 月營收列。
///
/// 欄位皆宣告為 `Option<Decimal>`：資料表雖為 NOT NULL，但 Option 解碼
/// 對非 NULL 值無額外成本，且與 `decimal_to_f64` 的簽名一致，未來 schema
/// 放寬也不會 panic。
#[derive(sqlx::FromRow)]
struct RevenueRow {
    /// 營收月份（`YYYYMM` 整數）。
    date: i64,
    /// 當月營收。
    monthly: Option<Decimal>,
    /// 上月營收。
    last_month: Option<Decimal>,
    /// 去年同月營收。
    last_year_this_month: Option<Decimal>,
    /// 本年度累計營收。
    monthly_accumulated: Option<Decimal>,
    /// 去年同期累計營收。
    last_year_monthly_accumulated: Option<Decimal>,
    /// 月增率。
    compared_with_last_month: Option<Decimal>,
    /// 年增率。
    compared_with_last_year_same_month: Option<Decimal>,
    /// 累計年增率。
    accumulated_compared_with_last_year: Option<Decimal>,
    /// 當月平均股價。
    avg_price: Option<Decimal>,
    /// 當月最低股價。
    lowest_price: Option<Decimal>,
    /// 當月最高股價。
    highest_price: Option<Decimal>,
}

/// 對應 `estimate` 個股估值列。
#[derive(sqlx::FromRow)]
struct ValuationRow {
    /// 股票代號。
    security_code: String,
    /// 估值日期。
    date: NaiveDate,
    /// 收盤價。
    closing_price: Decimal,
    /// 收盤價相對便宜價百分比。
    percentage: Decimal,
    /// 歷史樣本年度數。
    year_count: i32,
    /// 加權便宜價。
    cheap: Decimal,
    /// 加權合理價。
    fair: Decimal,
    /// 加權昂貴價。
    expensive: Decimal,
    /// 價格法便宜價。
    price_cheap: Decimal,
    /// 價格法合理價。
    price_fair: Decimal,
    /// 價格法昂貴價。
    price_expensive: Decimal,
    /// 股利法便宜價。
    dividend_cheap: Decimal,
    /// 股利法合理價。
    dividend_fair: Decimal,
    /// 股利法昂貴價。
    dividend_expensive: Decimal,
    /// EPS 法便宜價。
    eps_cheap: Decimal,
    /// EPS 法合理價。
    eps_fair: Decimal,
    /// EPS 法昂貴價。
    eps_expensive: Decimal,
    /// PBR 法便宜價。
    pbr_cheap: Decimal,
    /// PBR 法合理價。
    pbr_fair: Decimal,
    /// PBR 法昂貴價。
    pbr_expensive: Decimal,
    /// PER 法便宜價。
    per_cheap: Decimal,
    /// PER 法合理價。
    per_fair: Decimal,
    /// PER 法昂貴價。
    per_expensive: Decimal,
}

/// 將估值資料庫列轉成不暴露 Decimal 的 HTTP DTO。
impl From<ValuationRow> for StockValuation {
    fn from(row: ValuationRow) -> Self {
        let band = valuation_band(row.closing_price, row.cheap, row.fair, row.expensive);
        let symbol = row.security_code.clone();
        // 每個欄位都帶固定名稱進入轉換器，數值異常時能定位資料來源。
        let convert = |value, field| analytical_decimal_to_f64(Some(value), &symbol, field);
        Self {
            stock_symbol: row.security_code,
            date: row.date.to_string(),
            closing_price: convert(row.closing_price, "closing_price"),
            percentage: convert(row.percentage, "percentage"),
            year_count: row.year_count,
            cheap: convert(row.cheap, "cheap"),
            fair: convert(row.fair, "fair"),
            expensive: convert(row.expensive, "expensive"),
            price_cheap: convert(row.price_cheap, "price_cheap"),
            price_fair: convert(row.price_fair, "price_fair"),
            price_expensive: convert(row.price_expensive, "price_expensive"),
            dividend_cheap: convert(row.dividend_cheap, "dividend_cheap"),
            dividend_fair: convert(row.dividend_fair, "dividend_fair"),
            dividend_expensive: convert(row.dividend_expensive, "dividend_expensive"),
            eps_cheap: convert(row.eps_cheap, "eps_cheap"),
            eps_fair: convert(row.eps_fair, "eps_fair"),
            eps_expensive: convert(row.eps_expensive, "eps_expensive"),
            pbr_cheap: convert(row.pbr_cheap, "pbr_cheap"),
            pbr_fair: convert(row.pbr_fair, "pbr_fair"),
            pbr_expensive: convert(row.pbr_expensive, "pbr_expensive"),
            per_cheap: convert(row.per_cheap, "per_cheap"),
            per_fair: convert(row.per_fair, "per_fair"),
            per_expensive: convert(row.per_expensive, "per_expensive"),
            valuation_band: band.to_owned(),
        }
    }
}

/// 對應 `daily_stock_price_stats` 的單一市場統計列。
#[derive(sqlx::FromRow)]
struct MarketBreadthRow {
    /// 統計日期。
    date: NaiveDate,
    /// 低估家數。
    undervalued: i32,
    /// 合理估值家數。
    fair_valued: i32,
    /// 高估家數。
    overvalued: i32,
    /// 極高估家數。
    highly_overvalued: i32,
    /// 低於或等於五日線家數。
    below_5_day_moving_average: i32,
    /// 高於五日線家數。
    above_5_day_moving_average: i32,
    /// 低於或等於二十日線家數。
    below_20_day_moving_average: i32,
    /// 高於二十日線家數。
    above_20_day_moving_average: i32,
    /// 低於或等於六十日線家數。
    below_60_day_moving_average: i32,
    /// 高於六十日線家數。
    above_60_day_moving_average: i32,
    /// 低於或等於一百二十日線家數。
    below_120_day_moving_average: i32,
    /// 高於一百二十日線家數。
    above_120_day_moving_average: i32,
    /// 低於或等於二百四十日線家數。
    below_240_day_moving_average: i32,
    /// 高於二百四十日線家數。
    above_240_day_moving_average: i32,
    /// 上漲家數。
    stocks_up: i32,
    /// 下跌家數。
    stocks_down: i32,
    /// 平盤家數。
    stocks_unchanged: i32,
    /// 最後更新時間。
    updated_at: Option<DateTime<Utc>>,
}

impl MarketBreadthRow {
    /// 加入 API 使用的市場名稱並轉成市場廣度 DTO。
    fn into_dto(self, market: &str) -> MarketBreadth {
        MarketBreadth {
            date: self.date.to_string(),
            market: market.to_owned(),
            undervalued: self.undervalued,
            fair_valued: self.fair_valued,
            overvalued: self.overvalued,
            highly_overvalued: self.highly_overvalued,
            below_5_day_moving_average: self.below_5_day_moving_average,
            above_5_day_moving_average: self.above_5_day_moving_average,
            below_20_day_moving_average: self.below_20_day_moving_average,
            above_20_day_moving_average: self.above_20_day_moving_average,
            below_60_day_moving_average: self.below_60_day_moving_average,
            above_60_day_moving_average: self.above_60_day_moving_average,
            below_120_day_moving_average: self.below_120_day_moving_average,
            above_120_day_moving_average: self.above_120_day_moving_average,
            below_240_day_moving_average: self.below_240_day_moving_average,
            above_240_day_moving_average: self.above_240_day_moving_average,
            stocks_up: self.stocks_up,
            stocks_down: self.stocks_down,
            stocks_unchanged: self.stocks_unchanged,
            updated_at: timestamp(self.updated_at),
        }
    }
}

/// 對應殖利率排行 JOIN 後的資料庫列。
#[derive(sqlx::FromRow)]
struct YieldRankRow {
    /// 股票代號。
    stock_symbol: String,
    /// 股票名稱。
    name: String,
    /// 市場 id。
    market_id: i32,
    /// 產業 id。
    industry_id: i32,
    /// 排行日期。
    date: NaiveDate,
    /// 計算用收盤價。
    closing_price: Option<Decimal>,
    /// 計算用年度股利。
    dividend: Option<Decimal>,
    /// 殖利率百分比。
    dividend_yield_percent: Option<Decimal>,
}

impl YieldRankRow {
    /// 加入查詢結果順序產生的一起始名次並轉成排行 DTO。
    fn into_dto(self, rank: u32) -> DividendYieldRank {
        let symbol = self.stock_symbol.clone();
        DividendYieldRank {
            rank,
            stock_symbol: self.stock_symbol,
            name: self.name,
            market_id: self.market_id,
            industry_id: self.industry_id,
            date: self.date.to_string(),
            closing_price: analytical_decimal_to_f64(self.closing_price, &symbol, "closing_price"),
            dividend: analytical_decimal_to_f64(self.dividend, &symbol, "dividend"),
            dividend_yield_percent: analytical_decimal_to_f64(
                self.dividend_yield_percent,
                &symbol,
                "dividend_yield_percent",
            ),
        }
    }
}

/// 對應條件選股正規化後的資料庫列。
///
/// 指標與分類使用 `Option` 表達「來源不存在或已過期」；四個來源期間另行保留，
/// 因此呼叫端仍能辨識是完全沒有資料，或只是超過新鮮度上限。
#[derive(sqlx::FromRow)]
struct ScreenedStockRow {
    /// 股票代號。
    stock_symbol: String,
    /// 股票名稱。
    name: String,
    /// 上市或上櫃市場 id。
    market_id: i32,
    /// 產業分類 id。
    industry_id: i32,
    /// 新鮮營收年增率。
    revenue_yoy_percent: Option<Decimal>,
    /// 新鮮季度 EPS。
    earnings_per_share: Option<Decimal>,
    /// 新鮮季度 ROE。
    return_on_equity: Option<Decimal>,
    /// 新鮮殖利率。
    dividend_yield_percent: Option<Decimal>,
    /// 新鮮估值分類。
    valuation_band: Option<String>,
    /// 新鮮估值百分比。
    valuation_percentage: Option<Decimal>,
    /// 最新營收月份的資料庫 `YYYYMM` 編碼。
    revenue_month: Option<i64>,
    /// 最新季度財報年度。
    financial_year: Option<i64>,
    /// 最新季度財報季別。
    financial_quarter: Option<String>,
    /// 最新估值日期。
    valuation_date: Option<NaiveDate>,
    /// 最新殖利率日期。
    yield_date: Option<NaiveDate>,
}

impl From<ScreenedStockRow> for ScreenedStock {
    /// 將 SQL NULL 與來源期間轉成固定 API 契約。
    fn from(row: ScreenedStockRow) -> Self {
        let symbol = row.stock_symbol.clone();
        // 年度與季度必須同時存在才組成有效期間；LATERAL 財報列本身保證兩者
        // 同進同出，這個 zip 仍避免異常資料被輸出成不完整字串。
        let financial_period = row
            .financial_year
            .zip(row.financial_quarter)
            .map(|(year, quarter)| format!("{year}-{quarter}"));
        ScreenedStock {
            stock_symbol: row.stock_symbol,
            name: row.name,
            market_id: row.market_id,
            industry_id: row.industry_id,
            revenue_yoy_percent: analytical_decimal_to_f64(
                row.revenue_yoy_percent,
                &symbol,
                "revenue_yoy_percent",
            ),
            earnings_per_share: analytical_decimal_to_f64(
                row.earnings_per_share,
                &symbol,
                "earnings_per_share",
            ),
            return_on_equity: analytical_decimal_to_f64(
                row.return_on_equity,
                &symbol,
                "return_on_equity",
            ),
            dividend_yield_percent: analytical_decimal_to_f64(
                row.dividend_yield_percent,
                &symbol,
                "dividend_yield_percent",
            ),
            valuation_band: row.valuation_band,
            valuation_percentage: analytical_decimal_to_f64(
                row.valuation_percentage,
                &symbol,
                "valuation_percentage",
            ),
            revenue_month: row.revenue_month.map(format_month),
            financial_period,
            valuation_date: row.valuation_date.map(|date| date.to_string()),
            yield_date: row.yield_date.map(|date| date.to_string()),
        }
    }
}
impl RevenueRow {
    /// 將月營收列轉為 DTO，並讓每個 NUMERIC 欄位的轉換 log 帶股票代號。
    fn into_dto(self, symbol: &str) -> MonthlyRevenue {
        // 固定欄位名由程式碼提供，不能由呼叫端注入；資料異常時仍只輸出 null。
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        MonthlyRevenue {
            month: format_month(self.date),
            monthly_revenue: convert(self.monthly, "monthly_revenue"),
            last_month_revenue: convert(self.last_month, "last_month_revenue"),
            last_year_same_month_revenue: convert(
                self.last_year_this_month,
                "last_year_same_month_revenue",
            ),
            monthly_accumulated_revenue: convert(
                self.monthly_accumulated,
                "monthly_accumulated_revenue",
            ),
            last_year_monthly_accumulated_revenue: convert(
                self.last_year_monthly_accumulated,
                "last_year_monthly_accumulated_revenue",
            ),
            month_over_month_percent: convert(
                self.compared_with_last_month,
                "month_over_month_percent",
            ),
            year_over_year_percent: convert(
                self.compared_with_last_year_same_month,
                "year_over_year_percent",
            ),
            accumulated_year_over_year_percent: convert(
                self.accumulated_compared_with_last_year,
                "accumulated_year_over_year_percent",
            ),
            average_price: convert(self.avg_price, "average_price"),
            lowest_price: convert(self.lowest_price, "lowest_price"),
            highest_price: convert(self.highest_price, "highest_price"),
        }
    }
}

/// 對應 `financial_statement` 財報列。
#[derive(sqlx::FromRow)]
struct StatementRow {
    /// 財報西元年度。
    year: i64,
    /// 資料庫期間標記：空字串（年度）或 `Q1`–`Q4`。
    quarter: String,
    /// 毛利率。
    gross_profit: Option<Decimal>,
    /// 營業利益率。
    operating_profit_margin: Option<Decimal>,
    /// 稅前淨利率。
    pre_tax_income: Option<Decimal>,
    /// 稅後淨利率。
    net_income: Option<Decimal>,
    /// 每股淨值。
    net_asset_value_per_share: Option<Decimal>,
    /// 每股營收。
    sales_per_share: Option<Decimal>,
    /// 每股盈餘。
    earnings_per_share: Option<Decimal>,
    /// 每股稅前淨利。
    profit_before_tax: Option<Decimal>,
    /// 股東權益報酬率。
    return_on_equity: Option<Decimal>,
    /// 資產報酬率。
    return_on_assets: Option<Decimal>,
    /// 資料庫最後更新時間。
    updated_time: Option<DateTime<Utc>>,
}
impl StatementRow {
    /// 將財報列轉成 API DTO，年度空字串映射為 `A` 並保留轉換錯誤上下文。
    fn into_dto(self, symbol: &str) -> FinancialStatement {
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        FinancialStatement {
            year: self.year,
            quarter: quarter_to_api(&self.quarter),
            gross_profit_margin: convert(self.gross_profit, "gross_profit_margin"),
            operating_profit_margin: convert(
                self.operating_profit_margin,
                "operating_profit_margin",
            ),
            pre_tax_income_margin: convert(self.pre_tax_income, "pre_tax_income_margin"),
            net_income_margin: convert(self.net_income, "net_income_margin"),
            net_asset_value_per_share: convert(
                self.net_asset_value_per_share,
                "net_asset_value_per_share",
            ),
            sales_per_share: convert(self.sales_per_share, "sales_per_share"),
            earnings_per_share: convert(self.earnings_per_share, "earnings_per_share"),
            profit_before_tax_per_share: convert(
                self.profit_before_tax,
                "profit_before_tax_per_share",
            ),
            return_on_equity: convert(self.return_on_equity, "return_on_equity"),
            return_on_assets: convert(self.return_on_assets, "return_on_assets"),
            updated_at: timestamp(self.updated_time),
        }
    }
}

/// 對應 `dividend` 股利列。
#[derive(sqlx::FromRow)]
struct DividendRow {
    /// 發放年度（API `paid_year`）。
    year: i32,
    /// 股利所屬年度（API `dividend_year`）。
    year_of_dividend: i32,
    /// 資料庫期間標記：空字串（年度）、`H1`／`H2` 或 `Q1`–`Q4`。
    quarter: String,
    /// 現金股利合計。
    cash_dividend: Option<Decimal>,
    /// 股票股利合計。
    stock_dividend: Option<Decimal>,
    /// 現金與股票股利總和。
    sum: Option<Decimal>,
    /// 盈餘配息。
    earnings_cash_dividend: Option<Decimal>,
    /// 公積配息。
    capital_reserve_cash_dividend: Option<Decimal>,
    /// 盈餘配股。
    earnings_stock_dividend: Option<Decimal>,
    /// 公積配股。
    capital_reserve_stock_dividend: Option<Decimal>,
    /// 現金股利盈餘分配率。
    payout_ratio_cash: Option<Decimal>,
    /// 股票股利盈餘分配率。
    payout_ratio_stock: Option<Decimal>,
    /// 總盈餘分配率。
    payout_ratio: Option<Decimal>,
    /// 除息日（字串，可能是無效標記）。
    ex_dividend_date1: String,
    /// 除權日（字串，可能是無效標記）。
    ex_dividend_date2: String,
    /// 現金股利發放日（字串，可能是無效標記）。
    payable_date1: String,
    /// 股票股利發放日（字串，可能是無效標記）。
    payable_date2: String,
    /// 資料庫最後更新時間。
    updated_time: Option<DateTime<Utc>>,
}
impl DividendRow {
    /// 將股利列轉成 API DTO，清洗日期並以股票代號追蹤 NUMERIC 轉換失敗。
    fn into_dto(self, symbol: &str) -> Dividend {
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        Dividend {
            paid_year: self.year,
            dividend_year: self.year_of_dividend,
            quarter: quarter_to_api(&self.quarter),
            cash_dividend: convert(self.cash_dividend, "cash_dividend"),
            stock_dividend: convert(self.stock_dividend, "stock_dividend"),
            total_dividend: convert(self.sum, "total_dividend"),
            earnings_cash_dividend: convert(self.earnings_cash_dividend, "earnings_cash_dividend"),
            capital_reserve_cash_dividend: convert(
                self.capital_reserve_cash_dividend,
                "capital_reserve_cash_dividend",
            ),
            earnings_stock_dividend: convert(
                self.earnings_stock_dividend,
                "earnings_stock_dividend",
            ),
            capital_reserve_stock_dividend: convert(
                self.capital_reserve_stock_dividend,
                "capital_reserve_stock_dividend",
            ),
            cash_payout_ratio: convert(self.payout_ratio_cash, "cash_payout_ratio"),
            stock_payout_ratio: convert(self.payout_ratio_stock, "stock_payout_ratio"),
            total_payout_ratio: convert(self.payout_ratio, "total_payout_ratio"),
            ex_dividend_date: sanitize_date(&self.ex_dividend_date1),
            ex_rights_date: sanitize_date(&self.ex_dividend_date2),
            cash_payable_date: sanitize_date(&self.payable_date1),
            stock_payable_date: sanitize_date(&self.payable_date2),
            updated_at: timestamp(self.updated_time),
        }
    }
}

impl From<ProfileRow> for StockProfile {
    fn from(row: ProfileRow) -> Self {
        let history = row
            .maximum_price_date_on
            .is_some()
            .then(|| QuoteHistoryRecord {
                maximum_price: decimal_to_f64(row.maximum_price),
                maximum_price_date_on: row.maximum_price_date_on.map(|date| date.to_string()),
                minimum_price: decimal_to_f64(row.minimum_price),
                minimum_price_date_on: row.minimum_price_date_on.map(|date| date.to_string()),
                maximum_price_to_book_ratio: decimal_to_f64(row.maximum_price_to_book_ratio),
                maximum_price_to_book_ratio_date_on: row
                    .maximum_price_to_book_ratio_date_on
                    .map(|date| date.to_string()),
                minimum_price_to_book_ratio: decimal_to_f64(row.minimum_price_to_book_ratio),
                minimum_price_to_book_ratio_date_on: row
                    .minimum_price_to_book_ratio_date_on
                    .map(|date| date.to_string()),
            });
        Self {
            stock: row.latest.stock(),
            quote: row.latest.quote(),
            last_one_eps: decimal_to_f64(row.last_one_eps),
            last_four_eps: decimal_to_f64(row.last_four_eps),
            net_asset_value_per_share: decimal_to_f64(row.net_asset_value_per_share),
            return_on_equity: decimal_to_f64(row.return_on_equity),
            weight: decimal_to_f64(row.weight),
            issued_share: decimal_to_f64(row.issued_share),
            history,
        }
    }
}

#[cfg(test)]
mod tests {
    //! 純函式轉換邏輯的 deterministic tests（計畫 §9 Phase 1 要求）。
    //!
    //! 不需要資料庫：月份編碼、期間標記對映（§3.5）與股利日期清洗都是
    //! 純字串運算，行為必須與資料庫實際值域（P0-1～P0-3 實測）一一對應。

    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use super::{
        SCREEN_STOCKS_SQL, analytical_date_is_fresh, build_market_breadth_response,
        financial_period_is_fresh, format_month, market_id_for_stats, market_id_for_stocks,
        parse_month, parse_optional_date, quarter_to_api, revenue_month_is_fresh, sanitize_date,
        screen_order_by, validate_screening_params, valuation_band,
    };
    use crate::infra::database;
    use crate::interfaces::web::data_api::dto::{MarketBreadth, StockScreeningParams};

    /// 建立只覆寫測試重點欄位的 screen 預設參數。
    fn screening_params() -> StockScreeningParams {
        StockScreeningParams {
            market: None,
            industry_id: None,
            valuation_band: None,
            min_revenue_yoy_percent: None,
            min_eps: None,
            min_roe_percent: None,
            min_dividend_yield_percent: None,
            sort_by: None,
            sort_order: None,
            limit: None,
        }
    }

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

    /// 實際資料少於 `days` 時不補洞；首筆仍須同時作為 `breadth` 與
    /// `data_as_of`，空清單則由 handler 轉成 404。
    #[test]
    fn breadth_envelope_uses_first_available_history_row() {
        let point = MarketBreadth {
            date: "2026-07-16".to_owned(),
            market: "all".to_owned(),
            undervalued: 1,
            fair_valued: 2,
            overvalued: 3,
            highly_overvalued: 4,
            below_5_day_moving_average: 5,
            above_5_day_moving_average: 6,
            below_20_day_moving_average: 7,
            above_20_day_moving_average: 8,
            below_60_day_moving_average: 9,
            above_60_day_moving_average: 10,
            below_120_day_moving_average: 11,
            above_120_day_moving_average: 12,
            below_240_day_moving_average: 13,
            above_240_day_moving_average: 14,
            stocks_up: 15,
            stocks_down: 16,
            stocks_unchanged: 17,
            updated_at: None,
        };
        let response = build_market_breadth_response(vec![point.clone()]).unwrap();
        assert_eq!(response.data_as_of, "2026-07-16");
        assert_eq!(response.breadth, point);
        assert_eq!(response.breadth, response.history[0]);
        assert!(build_market_breadth_response(Vec::new()).is_none());
    }

    /// `all` 不縮小市場集合，不能單獨通過；上市／上櫃或任何一個實質條件
    /// 都可成立，固定預設排序與 limit 也應在驗證後補齊。
    #[test]
    fn screening_requires_a_meaningful_filter() {
        assert_eq!(
            validate_screening_params(&screening_params()).err(),
            Some("至少需要一個篩選條件")
        );
        let mut explicit_all = screening_params();
        explicit_all.market = Some("all".to_owned());
        assert!(validate_screening_params(&explicit_all).is_err());

        let mut twse = screening_params();
        twse.market = Some("twse".to_owned());
        let validated = validate_screening_params(&twse).expect("twse 本身是有效條件");
        assert_eq!(validated.market_id, 2);
        assert_eq!(validated.limit, 20);
        assert_eq!(validated.order_by, "ORDER BY stock_symbol ASC");

        let mut industry = screening_params();
        industry.industry_id = Some(24);
        assert!(validate_screening_params(&industry).is_ok());
        let mut minimum = screening_params();
        minimum.min_eps = Some(0.0);
        assert!(validate_screening_params(&minimum).is_ok());
    }

    /// 六個排序欄位乘上兩種方向必須完整映射，指標降冪也要明確
    /// `NULLS LAST`；不在白名單的文字必須在接觸 SQL 前遭拒。
    #[test]
    fn screening_sort_has_twelve_static_branches() {
        let fields = [
            "stock_symbol",
            "revenue_yoy",
            "eps",
            "roe",
            "dividend_yield",
            "valuation_percentage",
        ];
        for field in fields {
            for order in ["asc", "desc"] {
                let sql = screen_order_by(field, order).expect("白名單組合應存在");
                assert!(sql.starts_with("ORDER BY "));
                if field != "stock_symbol" {
                    assert!(sql.contains("NULLS LAST"));
                    assert!(sql.ends_with("stock_symbol ASC"));
                }
            }
        }
        assert!(screen_order_by("closing_price; DROP TABLE stocks", "asc").is_err());
        assert!(screen_order_by("eps", "sideways").is_err());
    }

    /// 數值範圍包含契約兩端，超界、NaN 與無限大都必須回 422；這可避免
    /// 非有限浮點數進入 Decimal 轉換或 PostgreSQL 比較。
    #[test]
    fn screening_numeric_boundaries_are_deterministic() {
        for value in [-100.0, 10_000.0] {
            let mut params = screening_params();
            params.min_revenue_yoy_percent = Some(value);
            assert!(validate_screening_params(&params).is_ok());
        }
        for value in [-100.01, 10_000.01, f64::NAN, f64::INFINITY] {
            let mut params = screening_params();
            params.min_revenue_yoy_percent = Some(value);
            assert!(validate_screening_params(&params).is_err());
        }
        let mut yield_at_zero = screening_params();
        yield_at_zero.min_dividend_yield_percent = Some(0.0);
        assert!(validate_screening_params(&yield_at_zero).is_ok());
        let mut negative_yield = screening_params();
        negative_yield.min_dividend_yield_percent = Some(-0.01);
        assert!(validate_screening_params(&negative_yield).is_err());
    }

    /// 新鮮度邊界使用月份／季度序號處理跨年，並以「差三月／兩季／三十日」
    /// 為仍有效的最後一天；未來日期一律不視為新鮮。
    #[test]
    fn screening_freshness_boundaries_cross_years() {
        let query_date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert!(revenue_month_is_fresh(query_date, 202510));
        assert!(!revenue_month_is_fresh(query_date, 202509));
        assert!(!revenue_month_is_fresh(query_date, 202602));

        assert!(financial_period_is_fresh(query_date, 2025, "Q3"));
        assert!(!financial_period_is_fresh(query_date, 2025, "Q2"));
        assert!(!financial_period_is_fresh(query_date, 2026, "Q2"));
        assert!(!financial_period_is_fresh(query_date, 2025, ""));

        let thirty_days_ago = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let thirty_one_days_ago = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        assert!(analytical_date_is_fresh(query_date, thirty_days_ago));
        assert!(!analytical_date_is_fresh(query_date, thirty_one_days_ago));
        assert!(!analytical_date_is_fresh(
            query_date,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
        ));
    }

    /// P0-4：以實際 PostgreSQL 對 Phase 2/3 核心查詢執行 EXPLAIN ANALYZE。
    ///
    /// 測試輸出文字 plan 供人工記錄 latency 與 scan 類型，並固定條件選股
    /// 四個「每股最新一期」子查詢使用既有索引；沒有資料庫時明確跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
    )]
    async fn phase2_phase3_query_plans_use_existing_indexes() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 P0-4 EXPLAIN：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let symbol: String = sqlx::query_scalar("SELECT stock_symbol FROM stocks WHERE stock_exchange_market_id IN (2,4) ORDER BY stock_symbol LIMIT 1")
            .fetch_one(pool).await.expect("應有上市櫃股票");

        // SQL 與 production 使用相同 WHERE、JOIN、排序及 LIMIT；EXPLAIN 不寫資料。
        let valuation: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT security_code, date, closing_price FROM estimate WHERE security_code = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30)) ORDER BY date DESC LIMIT 1"#)
            .bind(&symbol).bind(Option::<NaiveDate>::None).fetch_all(pool).await.expect("valuation EXPLAIN");
        let breadth: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) WITH endpoint AS (SELECT MAX(date) AS date FROM daily_stock_price_stats WHERE stock_exchange_market_id = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30))) SELECT s.date FROM daily_stock_price_stats s CROSS JOIN endpoint e WHERE s.stock_exchange_market_id = $1 AND s.date <= e.date ORDER BY s.date DESC LIMIT $3"#)
            .bind(0_i32).bind(Option::<NaiveDate>::None).bind(20_i64).fetch_all(pool).await.expect("breadth EXPLAIN");
        let ranking: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT y.security_code FROM yield_rank y JOIN stocks s ON s.stock_symbol = y.security_code JOIN "DailyQuotes" q ON q."Serial" = y.daily_quotes_serial JOIN dividend d ON d.serial = y.dividend_serial WHERE y.date = (SELECT MAX(date) FROM yield_rank) AND s.stock_exchange_market_id IN (2,4) ORDER BY y.yield DESC, y.security_code ASC LIMIT 20"#)
            .fetch_all(pool).await.expect("ranking EXPLAIN");
        let screen_sql = format!(
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {SCREEN_STOCKS_SQL}\nORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC\nLIMIT $9"
        );
        let screen: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(screen_sql.as_str()))
            .bind(0_i32)
            .bind(Option::<i32>::None)
            .bind(chrono::Local::now().date_naive())
            .bind(Option::<Decimal>::None)
            .bind(Option::<Decimal>::None)
            .bind(Option::<Decimal>::None)
            .bind(Option::<Decimal>::None)
            .bind(Option::<&str>::None)
            .bind(20_i64)
            .fetch_all(pool)
            .await
            .expect("screen EXPLAIN");

        for (name, plan) in [
            ("valuation", valuation),
            ("breadth", breadth),
            ("yield-ranking", ranking),
            ("screen", screen),
        ] {
            assert!(!plan.is_empty(), "{name} 應回傳 plan");
            println!("\n===== {name} =====\n{}", plan.join("\n"));
            if name == "screen" {
                assert!(
                    plan.join("\n")
                        .contains("yield_rank-security_code-date-desc-idx"),
                    "screen 每股最新殖利率必須走複合索引"
                );
            }
        }
    }
}
