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
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::dto::{
    DailyQuote, ErrorBody, HealthResponse, HistoricalQuote, HistoryParams, LatestQuoteResponse,
    PriceHistoryResponse, QuoteHistoryRecord, RealtimeSnapshotResponse, SearchParams,
    SearchResponse, Stock, StockProfile,
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
