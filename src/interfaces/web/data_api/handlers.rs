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
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::dto::{
    DailyQuote, Dividend, DividendHistoryParams, DividendHistoryResponse, ErrorBody,
    FinancialStatement, FinancialStatementHistoryResponse, HealthResponse, HistoricalQuote,
    HistoryParams, LatestQuoteResponse, MonthlyRevenue, MonthlyRevenueResponse,
    PriceHistoryResponse, QuoteHistoryRecord, RealtimeSnapshotResponse, RevenueHistoryParams,
    SearchParams, SearchResponse, StatementHistoryParams, Stock, StockProfile,
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
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/monthly-revenues", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), RevenueHistoryParams), responses((status = 200, body = MonthlyRevenueResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)), security(("bearer_auth" = [])))]
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
            Json(MonthlyRevenueResponse {
                stock_symbol: symbol,
                data_as_of,
                revenues: rows.into_iter().map(Into::into).collect(),
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
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/financial-statements", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), StatementHistoryParams), responses((status = 200, body = FinancialStatementHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)), security(("bearer_auth" = [])))]
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
            Json(FinancialStatementHistoryResponse {
                stock_symbol: symbol,
                data_as_of,
                statements: rows.into_iter().map(Into::into).collect(),
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
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/dividends", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), DividendHistoryParams), responses((status = 200, body = DividendHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)), security(("bearer_auth" = [])))]
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
            Json(DividendHistoryResponse {
                stock_symbol: symbol,
                data_as_of,
                dividends: rows.into_iter().map(Into::into).collect(),
            })
            .into_response()
        }
        Err(error) => database_error(error),
    }
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
    monthly: Option<Decimal>,
    last_month: Option<Decimal>,
    last_year_this_month: Option<Decimal>,
    monthly_accumulated: Option<Decimal>,
    last_year_monthly_accumulated: Option<Decimal>,
    compared_with_last_month: Option<Decimal>,
    compared_with_last_year_same_month: Option<Decimal>,
    accumulated_compared_with_last_year: Option<Decimal>,
    avg_price: Option<Decimal>,
    lowest_price: Option<Decimal>,
    highest_price: Option<Decimal>,
}
impl From<RevenueRow> for MonthlyRevenue {
    fn from(row: RevenueRow) -> Self {
        Self {
            month: format_month(row.date),
            monthly_revenue: decimal_to_f64(row.monthly),
            last_month_revenue: decimal_to_f64(row.last_month),
            last_year_same_month_revenue: decimal_to_f64(row.last_year_this_month),
            monthly_accumulated_revenue: decimal_to_f64(row.monthly_accumulated),
            last_year_monthly_accumulated_revenue: decimal_to_f64(
                row.last_year_monthly_accumulated,
            ),
            month_over_month_percent: decimal_to_f64(row.compared_with_last_month),
            year_over_year_percent: decimal_to_f64(row.compared_with_last_year_same_month),
            accumulated_year_over_year_percent: decimal_to_f64(
                row.accumulated_compared_with_last_year,
            ),
            average_price: decimal_to_f64(row.avg_price),
            lowest_price: decimal_to_f64(row.lowest_price),
            highest_price: decimal_to_f64(row.highest_price),
        }
    }
}

/// 對應 `financial_statement` 財報列。
#[derive(sqlx::FromRow)]
struct StatementRow {
    year: i64,
    /// 資料庫期間標記：空字串（年度）或 `Q1`–`Q4`。
    quarter: String,
    gross_profit: Option<Decimal>,
    operating_profit_margin: Option<Decimal>,
    pre_tax_income: Option<Decimal>,
    net_income: Option<Decimal>,
    net_asset_value_per_share: Option<Decimal>,
    sales_per_share: Option<Decimal>,
    earnings_per_share: Option<Decimal>,
    profit_before_tax: Option<Decimal>,
    return_on_equity: Option<Decimal>,
    return_on_assets: Option<Decimal>,
    updated_time: Option<DateTime<Utc>>,
}
impl From<StatementRow> for FinancialStatement {
    fn from(row: StatementRow) -> Self {
        Self {
            year: row.year,
            quarter: quarter_to_api(&row.quarter),
            gross_profit_margin: decimal_to_f64(row.gross_profit),
            operating_profit_margin: decimal_to_f64(row.operating_profit_margin),
            pre_tax_income_margin: decimal_to_f64(row.pre_tax_income),
            net_income_margin: decimal_to_f64(row.net_income),
            net_asset_value_per_share: decimal_to_f64(row.net_asset_value_per_share),
            sales_per_share: decimal_to_f64(row.sales_per_share),
            earnings_per_share: decimal_to_f64(row.earnings_per_share),
            profit_before_tax_per_share: decimal_to_f64(row.profit_before_tax),
            return_on_equity: decimal_to_f64(row.return_on_equity),
            return_on_assets: decimal_to_f64(row.return_on_assets),
            updated_at: timestamp(row.updated_time),
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
    cash_dividend: Option<Decimal>,
    stock_dividend: Option<Decimal>,
    sum: Option<Decimal>,
    earnings_cash_dividend: Option<Decimal>,
    capital_reserve_cash_dividend: Option<Decimal>,
    earnings_stock_dividend: Option<Decimal>,
    capital_reserve_stock_dividend: Option<Decimal>,
    payout_ratio_cash: Option<Decimal>,
    payout_ratio_stock: Option<Decimal>,
    payout_ratio: Option<Decimal>,
    /// 除息日（字串，可能是無效標記）。
    ex_dividend_date1: String,
    /// 除權日（字串，可能是無效標記）。
    ex_dividend_date2: String,
    /// 現金股利發放日（字串，可能是無效標記）。
    payable_date1: String,
    /// 股票股利發放日（字串，可能是無效標記）。
    payable_date2: String,
    updated_time: Option<DateTime<Utc>>,
}
impl From<DividendRow> for Dividend {
    fn from(row: DividendRow) -> Self {
        Self {
            paid_year: row.year,
            dividend_year: row.year_of_dividend,
            quarter: quarter_to_api(&row.quarter),
            cash_dividend: decimal_to_f64(row.cash_dividend),
            stock_dividend: decimal_to_f64(row.stock_dividend),
            total_dividend: decimal_to_f64(row.sum),
            earnings_cash_dividend: decimal_to_f64(row.earnings_cash_dividend),
            capital_reserve_cash_dividend: decimal_to_f64(row.capital_reserve_cash_dividend),
            earnings_stock_dividend: decimal_to_f64(row.earnings_stock_dividend),
            capital_reserve_stock_dividend: decimal_to_f64(row.capital_reserve_stock_dividend),
            cash_payout_ratio: decimal_to_f64(row.payout_ratio_cash),
            stock_payout_ratio: decimal_to_f64(row.payout_ratio_stock),
            total_payout_ratio: decimal_to_f64(row.payout_ratio),
            ex_dividend_date: sanitize_date(&row.ex_dividend_date1),
            ex_rights_date: sanitize_date(&row.ex_dividend_date2),
            cash_payable_date: sanitize_date(&row.payable_date1),
            stock_payable_date: sanitize_date(&row.payable_date2),
            updated_at: timestamp(row.updated_time),
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

    use super::{format_month, parse_month, quarter_to_api, sanitize_date};

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
}
