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
use chrono::{Local, NaiveDate};

use self::{
    params::{ValidatedScreeningParams, validate_screening_params},
    row::ScreenedStockRow,
    sql::{MAJOR_CHANGES_SQL, RANGE_BREAKOUTS_SQL, SCREEN_STOCKS_SQL},
};
use super::institutional_flow::{StreakRow, load_streaks};
use super::relative_strength::{ScoredStock, load_scores};
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
/// 財務體質（F 分數、毛利率、負債比、自由現金流）讀每日重算的 `financial_health`，營業利益率、
/// 稅後淨利率與三率三升取 Yahoo 最新一季單季損益表；
/// 營收動能（連續年增月數、近 3 月累計年增率）由最近 12 個月營收即時計算。
/// 相對強度分數與 `/market/relative-strength` 相同，以上市櫃普通股（不含 ETF、特別股）計算後再篩選；
/// 外資、投信連續買賣超天數與 `/market/institutional-flow` 同一算法，取最新籌碼資料日。
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

    // 查詢日由 handler 明確綁定，SQL 不直接讀 CURRENT_DATE；測試可傳入任意日期驗證
    // 月份、季度與 31 天邊界，production 則採伺服器台北本地日。
    screen_at(&params, validated, Local::now().date_naive()).await
}

/// 以指定查詢日執行已驗證的選股條件。
async fn screen_at(
    params: &StockScreeningParams,
    validated: ValidatedScreeningParams,
    query_date: NaiveDate,
) -> Response {
    // 相對強度要以上市櫃普通股（不含 ETF、特別股）排名、連續天數要回看多日籌碼，都無法在單股 LATERAL 內算，
    // 兩者同時先算好再以陣列傳入。
    let (scores, streaks, majors, breakouts) = tokio::join!(
        load_scores(query_date),
        load_streaks(query_date),
        load_major_changes(&validated, query_date),
        load_range_breakouts(&validated, query_date)
    );
    let (rs_as_of, rs_symbols, rs_scores) = match scores {
        Ok(scores) => rs_columns(scores),
        Err(response) => return *response,
    };
    let (chip_as_of, chip_symbols, foreign_streaks, trust_streaks) = match streaks {
        Ok(streaks) => streak_columns(streaks),
        Err(response) => return *response,
    };
    let (major_symbols, major_changes): (Vec<String>, Vec<rust_decimal::Decimal>) = match majors {
        Ok(rows) => rows.into_iter().unzip(),
        Err(error) => return database_error(error),
    };
    let mut breakout_symbols = Vec::new();
    let mut breakout_dates = Vec::new();
    let mut breakout_descriptions = Vec::new();
    match breakouts {
        Ok(rows) => {
            for (symbol, date, description) in rows {
                breakout_symbols.push(symbol);
                breakout_dates.push(date);
                breakout_descriptions.push(description);
            }
        }
        Err(error) => return database_error(error),
    }
    // ORDER BY 字串只可能來自下方三十二個 &'static str 分支，絕不插入呼叫端
    // 原始文字；其他條件仍全部使用 bind parameter。
    let sql = format!("{SCREEN_STOCKS_SQL}\n{}\nLIMIT $24", validated.order_by);
    // SQLx 0.9 要求動態字串顯式稽核；此處唯一動態部分已由
    // `screen_order_by` 限制為三十二個靜態常數，因此可安全標記。
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
        .bind(&rs_symbols)
        .bind(&rs_scores)
        .bind(validated.min_rs_score)
        .bind(&chip_symbols)
        .bind(&foreign_streaks)
        .bind(&trust_streaks)
        .bind(validated.min_foreign_streak)
        .bind(validated.min_trust_streak)
        .bind(i64::from(validated.limit))
        .bind(validated.min_avg_trade_value)
        .bind(validated.min_operating_margin_percent)
        .bind(params.margins_yoy_up.unwrap_or(false))
        .bind(params.revenue_record.unwrap_or(false))
        .bind(validated.exclude_industry_ids.as_deref())
        .bind(validated.max_pe)
        .bind(validated.max_pb)
        .bind(&major_symbols)
        .bind(&major_changes)
        .bind(validated.min_major_change)
        .bind(&breakout_symbols)
        .bind(&breakout_dates)
        .bind(&breakout_descriptions)
        .bind(validated.range_breakout_days.is_some())
        // 不快取成具名 prepared statement：同一連線執行約 5 次後 PostgreSQL 會改用 generic plan，
        // 34 個可選條件都剪不掉（2026-10-28 正式庫實測不設條件 custom 316 ms、generic 1,853 ms，
        // auto 模式第 6 次就換成 generic）。每次重新規劃只多 5 ms。
        .persistent(false)
        .fetch_all(database::get_connection())
        .await;

    match rows {
        Ok(rows) => Json(StockScreeningResponse {
            // 每股各來源可能來自不同日期，因此不可偽造單一 data_as_of。
            data_as_of: None,
            rs_as_of,
            chip_as_of,
            stocks: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 有設 `min_major_change` 時讀各股的千張大戶持股比例變化；沒有設時不查資料庫，回空清單。
async fn load_major_changes(
    validated: &ValidatedScreeningParams,
    query_date: NaiveDate,
) -> Result<Vec<(String, rust_decimal::Decimal)>, sqlx::Error> {
    if validated.min_major_change.is_none() {
        return Ok(Vec::new());
    }
    sqlx::query_as(MAJOR_CHANGES_SQL)
        .bind(query_date)
        .bind(i32::from(validated.major_change_weeks))
        .fetch_all(database::get_connection())
        .await
}

/// 有設 `range_breakout_days` 時讀近期還沒跌回的區間突破（代號、突破日、說明）；沒有設時不查資料庫。
async fn load_range_breakouts(
    validated: &ValidatedScreeningParams,
    query_date: NaiveDate,
) -> Result<Vec<(String, NaiveDate, String)>, sqlx::Error> {
    let Some(days) = validated.range_breakout_days else {
        return Ok(Vec::new());
    };
    sqlx::query_as(RANGE_BREAKOUTS_SQL)
        .bind(query_date)
        .bind(days)
        .fetch_all(database::get_connection())
        .await
}

/// 把全體計分結果拆成資料日與代號、分數兩個平行陣列；查無資料時兩個陣列皆空。
fn rs_columns(
    scores: Option<(NaiveDate, Vec<ScoredStock>)>,
) -> (Option<String>, Vec<String>, Vec<i16>) {
    let Some((data_date, scored)) = scores else {
        return (None, Vec::new(), Vec::new());
    };
    let (symbols, values) = scored
        .into_iter()
        .map(|stock| (stock.row.stock_symbol, i16::from(stock.rs_score)))
        .unzip();
    (Some(data_date.to_string()), symbols, values)
}

/// 籌碼連續天數拆成資料日與代號、外資、投信三個平行陣列；查無資料時陣列皆空。
fn streak_columns(
    streaks: Option<(NaiveDate, Vec<StreakRow>)>,
) -> (Option<String>, Vec<String>, Vec<i16>, Vec<i16>) {
    let Some((data_date, rows)) = streaks else {
        return (None, Vec::new(), Vec::new(), Vec::new());
    };
    let mut symbols = Vec::with_capacity(rows.len());
    let mut foreign = Vec::with_capacity(rows.len());
    let mut trust = Vec::with_capacity(rows.len());
    for row in rows {
        symbols.push(row.stock_symbol);
        foreign.push(row.foreign_streak);
        trust.push(row.trust_streak);
    }
    (Some(data_date.to_string()), symbols, foreign, trust)
}
