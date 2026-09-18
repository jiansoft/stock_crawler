//! `/market/dividend-yield-ranking` 與 `/market/qfii-holding-ranking`
//! 兩個排行 handlers、其排序白名單與對應的資料庫列型別。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{
    analytical_decimal_to_f64, database_error, error_response, market_id_for_stocks,
    parse_optional_date,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    DividendYieldRank, DividendYieldRankingParams, DividendYieldRankingResponse, ErrorBody,
    QfiiHolding, QfiiHoldingRankingParams, QfiiHoldingRankingResponse,
};

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
pub(crate) async fn dividend_yield_ranking(
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

/// 查詢外資（QFII）持股比例或持股數排行（§4.10）。
///
/// 這是**當前快照**：`stocks` 表只保存最近一次排程更新（每日 22:00 UTC）
/// 的數字，沒有歷史序列，無法回答「外資最近增減持」的趨勢問題。排除
/// 暫停上市（`"SuspendListing" = true`）與持股數為 0 的股票；市場條件同
/// §3.6，`all` 僅含上市＋上櫃。查無資料回 `200` 空陣列。
///
/// # Errors
///
/// `market`、`sort_by` 不在固定 enum、`industry_id` 非正整數或 `limit`
/// 超出 1–50 回 422；驗證失敗回 401；資料庫查詢失敗時記錄內部錯誤並回
/// 不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/qfii-holding-ranking", tag = "data-api", params(QfiiHoldingRankingParams), responses((status = 200, body = QfiiHoldingRankingResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn qfii_holding_ranking(
    Query(params): Query<QfiiHoldingRankingParams>,
) -> Response {
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
    let order_by = match qfii_order_by(params.sort_by.as_deref().unwrap_or("percentage")) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 50");
    }
    // ORDER BY 只可能來自 `qfii_order_by` 的兩個 &'static str 分支，呼叫端
    // 無法提供 SQL 片段；其餘條件全部使用 bind parameter。SQLx 0.9 對動態
    // 字串要求顯式稽核（AssertSqlSafe），與 screen_stocks 的做法一致。
    let sql = format!("{QFII_RANKING_SQL}\n{order_by}\nLIMIT $3");
    let rows: Result<Vec<QfiiHoldingRow>, _> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(market_id)
        .bind(params.industry_id)
        .bind(i64::from(limit))
        .fetch_all(database::get_connection())
        .await;
    match rows {
        Ok(rows) => Json(QfiiHoldingRankingResponse {
            // `stocks` 表快照沒有列級更新日期，依 §4.10 不可偽造一個。
            data_as_of: None,
            stocks: rows
                .into_iter()
                .enumerate()
                // 名次由查詢結果順序在程式端產生（一起始），與殖利率排行一致。
                .map(|(index, row)| row.into_dto(index as u32 + 1))
                .collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// QFII 排行的參數化 SQL 主體（不含排序與 LIMIT）。
///
/// 綁定參數：
/// - `$1`：市場 id；`0` 是 API 內部的 all 哨兵，只展開為固定 `IN (2, 4)`
///   （上市＋上櫃，刻意排除公開發行與興櫃，§3.6）。
/// - `$2`：可選產業 id；NULL 時不過濾。
/// - `$3`：回傳筆數上限（附加於排序分支之後）。
///
/// 排除條件寫死在 SQL：暫停上市股票的快照數字已無參考價值；持股數 0 多為
/// 外資從未進場或資料未更新的股票，混入排行尾端沒有意義。stocks 表僅數千
/// 列，P0-4 EXPLAIN 實測 Seq Scan + top-N 排序約 1.8ms，無需索引。
const QFII_RANKING_SQL: &str = r#"
SELECT stock_symbol, "Name" AS name,
       stock_exchange_market_id AS market_id, stock_industry_id AS industry_id,
       qfii_shares_held, qfii_share_holding_percentage, issued_share
FROM stocks
WHERE (($1 = 0 AND stock_exchange_market_id IN (2, 4))
    OR stock_exchange_market_id = $1)
  AND ($2::int IS NULL OR stock_industry_id = $2)
  AND "SuspendListing" = false
  AND qfii_shares_held <> 0
"#;

/// 將 QFII 排行的排序 enum 映射成兩個固定 SQL 分支（§4.10）。
///
/// 與 `screen_order_by` 相同的白名單做法：回傳值只可能是程式內
/// `&'static str`，呼叫端文字絕不進入 SQL。兩個指標欄位在資料庫皆為
/// NOT NULL（預設 0），不需 `NULLS LAST`；一律由高到低，同值以股票代號
/// 升冪穩定排序，使重複查詢的名次不漂移。
fn qfii_order_by(sort_by: &str) -> Result<&'static str, &'static str> {
    match sort_by {
        "percentage" => Ok("ORDER BY qfii_share_holding_percentage DESC, stock_symbol ASC"),
        "shares" => Ok("ORDER BY qfii_shares_held DESC, stock_symbol ASC"),
        _ => Err("sort_by 必須為 percentage 或 shares"),
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

/// 對應 QFII 排行的 `stocks` 快照列（§4.10）。
#[derive(sqlx::FromRow)]
struct QfiiHoldingRow {
    /// 股票代號。
    stock_symbol: String,
    /// 股票名稱。
    name: String,
    /// 市場 id（上市 2、上櫃 4）。
    market_id: i32,
    /// 產業分類 id。
    industry_id: i32,
    /// 外資及陸資持有股數（bigint，直接以 i64 輸出，不經浮點轉換）。
    qfii_shares_held: i64,
    /// 外資及陸資持股比率（NUMERIC(18,4)）。
    qfii_share_holding_percentage: Option<Decimal>,
    /// 發行股數（bigint）。
    issued_share: i64,
}

impl QfiiHoldingRow {
    /// 加入查詢結果順序產生的一起始名次並轉成排行 DTO。
    fn into_dto(self, rank: u32) -> QfiiHolding {
        let symbol = self.stock_symbol.clone();
        QfiiHolding {
            rank,
            stock_symbol: self.stock_symbol,
            name: self.name,
            market_id: self.market_id,
            industry_id: self.industry_id,
            qfii_shares_held: self.qfii_shares_held,
            qfii_share_holding_percentage: analytical_decimal_to_f64(
                self.qfii_share_holding_percentage,
                &symbol,
                "qfii_share_holding_percentage",
            ),
            issued_share: self.issued_share,
        }
    }
}

#[cfg(test)]
mod tests {
    //! 排行排序白名單的 deterministic tests。

    use super::qfii_order_by;

    /// §4.10 QFII 排序白名單：兩個固定分支皆為降冪加股票代號穩定排序；
    /// 白名單以外的文字必須在接觸 SQL 前遭拒。
    #[test]
    fn qfii_sort_has_two_static_branches() {
        assert_eq!(
            qfii_order_by("percentage"),
            Ok("ORDER BY qfii_share_holding_percentage DESC, stock_symbol ASC")
        );
        assert_eq!(
            qfii_order_by("shares"),
            Ok("ORDER BY qfii_shares_held DESC, stock_symbol ASC")
        );
        for invalid in ["", "issued_share", "percentage; DROP TABLE stocks"] {
            assert!(qfii_order_by(invalid).is_err(), "{invalid:?} 應被拒絕");
        }
    }
}
