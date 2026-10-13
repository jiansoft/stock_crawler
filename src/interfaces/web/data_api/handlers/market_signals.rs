//! `/market/signals` handler：全市場技術訊號選股。
//!
//! 訊號在收盤匯總時預先算好寫入 `daily_technical_signal`（見
//! [`crate::app::calculation::technical_signal`]），這裡只查表並附上當日報價。

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{
    database_error, decimal_to_f64, error_response, market_id_for_stocks, parse_optional_date,
};
use crate::domain::quote::technical::{SIGNAL_INDICATORS, SignalKind};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketSignalsParams, MarketSignalsResponse, SignalCount, SignalStock,
};

/// `$1`（含）之前最近一個有訊號的交易日；`$1` 為 `NULL` 時取最新。
const DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_technical_signal WHERE $1::date IS NULL OR date <= $1
"#;

/// 資料日的訊號，附股票、產業與當日報價。`$2 = 0` 是 all 的哨兵（訊號只算上市櫃）。
const ROWS_SQL: &str = r#"
SELECT
    t.stock_symbol,
    s."Name" AS name,
    s.stock_exchange_market_id AS market_id,
    COALESCE(si.name, '') AS industry,
    q."ClosingPrice" AS closing_price,
    q."ChangeRange" AS change_range,
    q."TradeValue" AS trade_value,
    t.indicator,
    t.kind,
    t.description
FROM daily_technical_signal t
JOIN stocks s ON s.stock_symbol = t.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN "DailyQuotes" q ON q.stock_symbol = t.stock_symbol AND q."Date" = t.date
WHERE t.date = $1
  AND ($2 = 0 OR s.stock_exchange_market_id = $2)
  AND ($3::text IS NULL OR si.name = $3)
"#;

/// 單筆訊號與當日報價。
#[derive(Debug, Clone, sqlx::FromRow)]
struct SignalRow {
    stock_symbol: String,
    name: String,
    market_id: i32,
    industry: String,
    closing_price: Option<Decimal>,
    change_range: Option<Decimal>,
    trade_value: Option<Decimal>,
    indicator: String,
    kind: String,
    description: String,
}

/// 查詢指定交易日出現技術訊號的股票。
///
/// 訊號種類：均線／KD／MACD 的黃金交叉與死亡交叉、收盤價站上或跌破月季年線、
/// RSI(14) 與 K 值超買超賣、52 週新高新低。價格未還原除權息。
///
/// # Errors
///
/// 參數格式錯誤、`kind`／`indicator` 不在固定清單、`limit` 超出 1–200 或產業名稱不存在回 422；
/// 指定日期之前沒有任何訊號資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/signals", tag = "data-api", params(MarketSignalsParams), responses((status = 200, body = MarketSignalsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_signals(Query(params): Query<MarketSignalsParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    if params
        .kind
        .as_deref()
        .is_some_and(|kind| SignalKind::parse(kind).is_none())
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "kind 必須為 golden_cross、death_cross、cross_above、cross_below、overbought、oversold、new_high 或 new_low",
        );
    }
    if params
        .indicator
        .as_deref()
        .is_some_and(|indicator| !SIGNAL_INDICATORS.contains(&indicator))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "indicator 必須為 ma5_ma20、ma20_ma60、kd、macd、close_ma20、close_ma60、close_ma240、rsi14 或 52_week",
        );
    }
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }

    let pool = database::get_connection();
    if let Some(industry) = params.industry.as_deref() {
        let found: Result<Option<(i32,)>, _> =
            sqlx::query_as("SELECT stock_industry_id FROM stock_industry WHERE name = $1")
                .bind(industry)
                .fetch_optional(pool)
                .await;
        match found {
            Ok(Some(_)) => {}
            Ok(None) => {
                return error_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "industry 必須為產業名稱，例如「半導體業」",
                );
            }
            Err(error) => return database_error(error),
        }
    }

    let data_date: Option<NaiveDate> = match sqlx::query_scalar(DATE_SQL)
        .bind(date)
        .fetch_one(pool)
        .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let Some(data_date) = data_date else {
        return error_response(StatusCode::NOT_FOUND, "查無技術訊號資料");
    };
    let rows: Vec<SignalRow> = match sqlx::query_as(ROWS_SQL)
        .bind(data_date)
        .bind(market_id)
        .bind(params.industry.as_deref())
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    Json(build_response(
        data_date,
        market,
        &params,
        usize::from(limit),
        rows,
    ))
    .into_response()
}

/// 統計各來源與種類的家數，再依 `kind`／`indicator` 篩選、依成交值排序後取前 `limit` 筆。
fn build_response(
    data_date: NaiveDate,
    market: &str,
    params: &MarketSignalsParams,
    limit: usize,
    rows: Vec<SignalRow>,
) -> MarketSignalsResponse {
    let mut counts: BTreeMap<(usize, usize), (String, String, usize)> = BTreeMap::new();
    for row in &rows {
        let key = (indicator_order(&row.indicator), kind_order(&row.kind));
        counts
            .entry(key)
            .or_insert_with(|| (row.indicator.clone(), row.kind.clone(), 0))
            .2 += 1;
    }
    let summary = counts
        .into_values()
        .map(|(indicator, kind, count)| SignalCount {
            indicator,
            kind,
            count,
        })
        .collect();

    let mut matched: Vec<SignalRow> = rows
        .into_iter()
        .filter(|row| params.kind.as_deref().is_none_or(|kind| row.kind == kind))
        .filter(|row| {
            params
                .indicator
                .as_deref()
                .is_none_or(|indicator| row.indicator == indicator)
        })
        .collect();
    matched.sort_by(|a, b| {
        b.trade_value
            .unwrap_or_default()
            .cmp(&a.trade_value.unwrap_or_default())
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
            .then_with(|| indicator_order(&a.indicator).cmp(&indicator_order(&b.indicator)))
            .then_with(|| kind_order(&a.kind).cmp(&kind_order(&b.kind)))
    });
    let total = matched.len();
    let items = matched.into_iter().take(limit).map(to_item).collect();

    MarketSignalsResponse {
        data_as_of: data_date.to_string(),
        market: market.to_owned(),
        kind: params.kind.clone(),
        indicator: params.indicator.clone(),
        industry: params.industry.clone(),
        adjusted: false,
        total,
        summary,
        items,
    }
}

/// 訊號來源在對外說明中的順序；未知來源排最後。
fn indicator_order(indicator: &str) -> usize {
    SIGNAL_INDICATORS
        .iter()
        .position(|known| *known == indicator)
        .unwrap_or(usize::MAX)
}

/// 訊號種類在對外說明中的順序；未知種類排最後。
fn kind_order(kind: &str) -> usize {
    SignalKind::parse(kind).map_or(usize::MAX, |kind| kind as usize)
}

/// 轉成對外的訊號股票。
fn to_item(row: SignalRow) -> SignalStock {
    SignalStock {
        market: if row.market_id == 4 { "tpex" } else { "twse" }.to_owned(),
        closing_price: decimal_to_f64(row.closing_price),
        change_percent: decimal_to_f64(row.change_range),
        trade_value: decimal_to_f64(row.trade_value),
        stock_symbol: row.stock_symbol,
        name: row.name,
        industry: row.industry,
        indicator: row.indicator,
        kind: row.kind,
        description: row.description,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(symbol: &str, indicator: &str, kind: &str, trade_value: Option<i64>) -> SignalRow {
        SignalRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol}名稱"),
            market_id: if symbol.ends_with('T') { 4 } else { 2 },
            industry: "半導體業".to_owned(),
            closing_price: Some(Decimal::new(1234, 1)),
            change_range: Some(Decimal::new(-15, 1)),
            trade_value: trade_value.map(Decimal::from),
            indicator: indicator.to_owned(),
            kind: kind.to_owned(),
            description: format!("{symbol} {indicator} {kind}"),
        }
    }

    fn params(kind: Option<&str>, indicator: Option<&str>) -> MarketSignalsParams {
        MarketSignalsParams {
            date: None,
            market: None,
            kind: kind.map(str::to_owned),
            indicator: indicator.map(str::to_owned),
            industry: Some("半導體業".to_owned()),
            limit: None,
        }
    }

    fn rows() -> Vec<SignalRow> {
        vec![
            row("A", "kd", "overbought", Some(100)),
            row("B", "52_week", "new_high", Some(500)),
            row("CT", "kd", "golden_cross", None),
            row("C", "kd", "golden_cross", Some(500)),
            row("A", "kd", "golden_cross", Some(100)),
        ]
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期")
    }

    /// 家數依來源、種類的對外順序統計，不受篩選影響；項目依成交值排序並套用 limit。
    #[test]
    fn build_response_counts_filters_and_sorts() {
        let response = build_response(
            date(),
            "all",
            &params(Some("golden_cross"), None),
            2,
            rows(),
        );
        assert_eq!(response.data_as_of, "2026-10-12");
        assert_eq!(response.kind.as_deref(), Some("golden_cross"));
        assert_eq!(response.industry.as_deref(), Some("半導體業"));
        assert!(!response.adjusted);
        let summary: Vec<(&str, &str, usize)> = response
            .summary
            .iter()
            .map(|c| (c.indicator.as_str(), c.kind.as_str(), c.count))
            .collect();
        assert_eq!(
            summary,
            [
                ("kd", "golden_cross", 3),
                ("kd", "overbought", 1),
                ("52_week", "new_high", 1)
            ]
        );
        assert_eq!(response.total, 3);
        let symbols: Vec<&str> = response
            .items
            .iter()
            .map(|item| item.stock_symbol.as_str())
            .collect();
        assert_eq!(symbols, ["C", "A"]);
        let first = &response.items[0];
        assert_eq!(first.market, "twse");
        assert_eq!(first.closing_price, Some(123.4));
        assert_eq!(first.change_percent, Some(-1.5));
        assert_eq!(first.trade_value, Some(500.0));
        assert_eq!(first.description, "C kd golden_cross");
    }

    /// 依來源篩選；沒有成交值的排在最後，上櫃股票標為 tpex。
    #[test]
    fn build_response_filters_by_indicator() {
        let response = build_response(date(), "all", &params(None, Some("kd")), 50, rows());
        let items: Vec<(&str, &str)> = response
            .items
            .iter()
            .map(|item| (item.stock_symbol.as_str(), item.kind.as_str()))
            .collect();
        assert_eq!(
            items,
            [
                ("C", "golden_cross"),
                ("A", "golden_cross"),
                ("A", "overbought"),
                ("CT", "golden_cross")
            ]
        );
        assert_eq!(response.items[3].market, "tpex");
        assert_eq!(response.items[3].trade_value, None);
    }

    /// 未知的來源或種類（資料表手動寫入的舊值）排在最後，不會讓排序失敗。
    #[test]
    fn unknown_names_sort_last() {
        assert_eq!(indicator_order("unknown"), usize::MAX);
        assert_eq!(kind_order("unknown"), usize::MAX);
        assert_eq!(indicator_order("ma5_ma20"), 0);
        assert_eq!(kind_order("new_low"), 7);
    }
}
