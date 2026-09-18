//! `/market/breadth` 與 `/market/index-history` 兩個市場統計 handlers
//! 及其對應的資料庫列型別。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::{
    analytical_decimal_to_f64, database_error, error_response, market_id_for_stats,
    parse_optional_date, timestamp,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketBreadth, MarketBreadthParams, MarketBreadthResponse, MarketIndexHistoryParams,
    MarketIndexHistoryResponse, MarketIndexPoint,
};

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
pub(crate) async fn market_breadth(Query(params): Query<MarketBreadthParams>) -> Response {
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

/// 查詢台股大盤指數（TAIEX）歷史走勢（§4.8）。
///
/// endpoint 固定查 `index` 表 `category = 'TAIEX'`，不開放指數類別參數；
/// 未來若要支援其他指數，以新增選填 query 參數的向後相容方式擴充。查無
/// 資料回 `200` 空陣列——大盤指數沒有「代號不存在」的 404 語意。
///
/// # Errors
///
/// 日期格式錯誤、`from` 晚於 `to` 或 `limit` 超出 1–365 回 422；驗證失敗
/// 回 401；資料庫查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/index-history", tag = "data-api", params(MarketIndexHistoryParams), responses((status = 200, body = MarketIndexHistoryResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_index_history(
    Query(params): Query<MarketIndexHistoryParams>,
) -> Response {
    // 與其他分析 endpoints 一致採嚴格 `YYYY-MM-DD` 驗證（拒絕未補零），
    // 避免 PostgreSQL 寬鬆轉型讓不同 endpoint 行為不一致。
    let from = match parse_optional_date(params.from.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let to = match parse_optional_date(params.to.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if from.zip(to).is_some_and(|(start, end)| start > end) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "from 不可晚於 to");
    }
    // 上限 365 與 `price-history` 慣例一致：一年份日線已足夠趨勢判讀，
    // 也避免 LLM 一次取得過多 token。
    let limit = params.limit.unwrap_or(30);
    if !(1..=365).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 365");
    }
    // `'TAIEX'` 是程式內固定字面值（契約規定），不是呼叫端輸入；
    // `index-date_category-uidx` 以 (date, category) 為鍵，P0-4 已驗證此
    // 查詢走索引反向掃描（0.06ms）。`$1/$2` 為 NULL 時代表不限制區間。
    let rows: Result<Vec<IndexPointRow>, _> = sqlx::query_as(r#"SELECT "date", index, change, trade_value, "transaction", trading_volume FROM index WHERE category = 'TAIEX' AND ($1::date IS NULL OR "date" >= $1) AND ($2::date IS NULL OR "date" <= $2) ORDER BY "date" DESC LIMIT $3"#).bind(from).bind(to).bind(i64::from(limit)).fetch_all(database::get_connection()).await;
    match rows {
        Ok(rows) => {
            // 清單固定新到舊（§3.1），第一筆即最新一筆；空清單時
            // `data_as_of` 維持 null，不揣測日期。
            let data_as_of = rows.first().map(|row| row.date.to_string());
            Json(MarketIndexHistoryResponse {
                data_as_of,
                points: rows.into_iter().map(Into::into).collect(),
            })
            .into_response()
        }
        Err(error) => database_error(error),
    }
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

/// 對應 `index` 表的大盤指數列（§4.8）。
///
/// 欄位皆宣告為 `Option<Decimal>`：資料表雖為 NOT NULL（預設 0），Option
/// 解碼對非 NULL 值無額外成本，且與轉換函式簽名一致，未來 schema 放寬也
/// 不會 panic。
#[derive(sqlx::FromRow)]
struct IndexPointRow {
    /// 指數日期。
    date: NaiveDate,
    /// 收盤指數。
    index: Option<Decimal>,
    /// 漲跌點數。
    change: Option<Decimal>,
    /// 成交金額（元）。
    trade_value: Option<Decimal>,
    /// 成交筆數。
    transaction: Option<Decimal>,
    /// 成交股數。
    trading_volume: Option<Decimal>,
}

impl From<IndexPointRow> for MarketIndexPoint {
    /// 將指數列轉為 API DTO；NUMERIC 轉換失敗時以固定標記 `TAIEX` 記錄
    /// log（大盤指數沒有股票代號，用類別代碼替代）並輸出 `null`。
    fn from(row: IndexPointRow) -> Self {
        let convert = |value, field| analytical_decimal_to_f64(value, "TAIEX", field);
        Self {
            date: row.date.to_string(),
            index: convert(row.index, "index"),
            change: convert(row.change, "change"),
            trade_value: convert(row.trade_value, "trade_value"),
            transaction: convert(row.transaction, "transaction"),
            trading_volume: convert(row.trading_volume, "trading_volume"),
        }
    }
}

#[cfg(test)]
mod tests {
    //! 市場統計 envelope 組裝的 deterministic tests。

    use super::build_market_breadth_response;
    use crate::interfaces::web::data_api::dto::MarketBreadth;

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
}
