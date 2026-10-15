//! `/market/industry-flow` handler：三大法人資金在各產業的淨流入流出。
//!
//! 個股淨買賣沿用 [`super::institutional_flow`] 的期間累計（估計金額＝每日淨股數 × 當日收盤價），
//! 依產業加總；另附買超／賣超家數、淨額占產業成交金額的比例與買賣超金額最大的個股。

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Local;

use super::institutional_flow::{FlowPeriod, FlowRow, INVESTORS, MAX_FLOW_DAYS, load_flows};
use super::{error_response, market_id_for_stocks, parse_optional_date, round2};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, IndustryFlowItem, IndustryFlowParams, IndustryFlowResponse, IndustryFlowStock,
};

/// 每個產業列出的買超、賣超個股檔數。
const TOP_STOCKS: usize = 3;

/// 查詢各產業的法人淨買賣金額。
///
/// 累計最近 `days` 個有籌碼資料的交易日；未分類的股票只計入 `total_net_amount`，不列成產業。
///
/// # Errors
///
/// 參數格式錯誤、`days` 超出 1–60、`investor`／`market`／`order` 不合法回 422；查無籌碼資料回 404；
/// 驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/industry-flow", tag = "data-api", params(IndustryFlowParams), responses((status = 200, body = IndustryFlowResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn industry_flow(Query(params): Query<IndustryFlowParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(5);
    if !(1..=MAX_FLOW_DAYS).contains(&usize::from(days)) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }
    let investor = params.investor.as_deref().unwrap_or("foreign");
    if !INVESTORS.contains(&investor) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "investor 必須為 foreign、trust、dealer 或 total",
        );
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let order = params.order.as_deref().unwrap_or("desc");
    if !matches!(order, "asc" | "desc") {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "order 必須為 asc 或 desc");
    }

    let until = date.unwrap_or_else(|| Local::now().date_naive());
    match load_flows(until, usize::from(days), investor).await {
        Ok(Some((period, rows))) => Json(build_response(
            period, investor, market, market_id, order, rows,
        ))
        .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無籌碼資料"),
        Err(response) => *response,
    }
}

/// 依市場篩選後依產業加總，依淨買賣金額排名（同金額依產業名稱），`asc` 時反轉輸出順序。
fn build_response(
    period: FlowPeriod,
    investor: &str,
    market: &str,
    market_id: i32,
    order: &str,
    rows: Vec<FlowRow>,
) -> IndustryFlowResponse {
    let mut total_net_amount = 0.0;
    let mut groups: BTreeMap<String, Vec<FlowRow>> = BTreeMap::new();
    for row in rows {
        if market_id != 0 && row.market_id != market_id {
            continue;
        }
        total_net_amount += row.net_amount;
        if !row.industry.is_empty() {
            groups.entry(row.industry.clone()).or_default().push(row);
        }
    }

    let mut items: Vec<IndustryFlowItem> = groups
        .into_iter()
        .map(|(industry, stocks)| industry_item(industry, stocks))
        .collect();
    items.sort_by(|a, b| {
        b.net_amount
            .total_cmp(&a.net_amount)
            .then_with(|| a.industry.cmp(&b.industry))
    });
    for (index, item) in items.iter_mut().enumerate() {
        item.rank = index + 1;
    }
    if order == "asc" {
        items.reverse();
    }

    IndustryFlowResponse {
        data_as_of: period.data_date.to_string(),
        from: period.from.to_string(),
        days: period.days,
        investor: investor.to_owned(),
        market: market.to_owned(),
        order: order.to_owned(),
        total_net_amount: total_net_amount.round(),
        items,
    }
}

/// 單一產業的統計：家數、淨額、占成交金額比例與買賣超金額最大的個股（同金額依代號）。
fn industry_item(industry: String, mut stocks: Vec<FlowRow>) -> IndustryFlowItem {
    let net_amount: f64 = stocks.iter().map(|row| row.net_amount).sum();
    let trade_value: f64 = stocks.iter().filter_map(|row| row.trade_value).sum();
    stocks.sort_by(|a, b| {
        b.net_amount
            .total_cmp(&a.net_amount)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    let top = |row: &FlowRow| IndustryFlowStock {
        stock_symbol: row.stock_symbol.clone(),
        name: row.name.clone(),
        net_amount: row.net_amount.round(),
    };
    IndustryFlowItem {
        rank: 0,
        stocks: stocks.len(),
        buy_stocks: stocks.iter().filter(|row| row.net_amount > 0.0).count(),
        sell_stocks: stocks.iter().filter(|row| row.net_amount < 0.0).count(),
        net_amount: net_amount.round(),
        trade_value_ratio: (trade_value > 0.0).then(|| round2(net_amount / trade_value * 100.0)),
        top_buyers: stocks
            .iter()
            .filter(|row| row.net_amount > 0.0)
            .take(TOP_STOCKS)
            .map(top)
            .collect(),
        top_sellers: stocks
            .iter()
            .rev()
            .filter(|row| row.net_amount < 0.0)
            .take(TOP_STOCKS)
            .map(top)
            .collect(),
        industry,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
    }

    fn row(symbol: &str, market_id: i32, industry: &str, amount: f64) -> FlowRow {
        FlowRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol}名稱"),
            market_id,
            industry: industry.to_owned(),
            net_shares: 0,
            net_amount: amount,
            volume: None,
            trade_value: Some(1_000_000.0),
            buy_days: 0,
            sell_days: 0,
            streak: 0,
            closing_price: None,
        }
    }

    fn period() -> FlowPeriod {
        FlowPeriod {
            data_date: day(13),
            from: day(7),
            days: 5,
        }
    }

    fn universe() -> Vec<FlowRow> {
        vec![
            row("A", 2, "半導體業", 300_000.0),
            row("B", 4, "半導體業", -50_000.0),
            row("C", 2, "半導體業", 100_000.0),
            row("D", 2, "半導體業", 100_000.0),
            row("E", 2, "半導體業", 20_000.0),
            row("F", 2, "航運業", -200_000.0),
            row("G", 4, "航運業", -100_000.4),
            row("H", 2, "", 7_000.0),
        ]
    }

    /// 產業加總、家數、占比、買賣超個股（各最多 3 檔、同金額依代號）、未分類只計入合計、名次。
    #[test]
    fn build_response_aggregates_industries() {
        let response = build_response(period(), "trust", "all", 0, "desc", universe());
        assert_eq!(response.data_as_of, "2026-10-13");
        assert_eq!(response.from, "2026-10-07");
        assert_eq!(response.days, 5);
        assert_eq!(response.investor, "trust");
        assert_eq!(response.order, "desc");
        assert_eq!(response.total_net_amount, 177_000.0);
        let names: Vec<(usize, &str)> = response
            .items
            .iter()
            .map(|item| (item.rank, item.industry.as_str()))
            .collect();
        assert_eq!(names, [(1, "半導體業"), (2, "航運業")]);

        let semi = &response.items[0];
        assert_eq!((semi.stocks, semi.buy_stocks, semi.sell_stocks), (5, 4, 1));
        assert_eq!(semi.net_amount, 470_000.0);
        assert_eq!(semi.trade_value_ratio, Some(9.4));
        let buyers: Vec<&str> = semi
            .top_buyers
            .iter()
            .map(|stock| stock.stock_symbol.as_str())
            .collect();
        assert_eq!(buyers, ["A", "C", "D"]);
        assert_eq!(semi.top_buyers[0].name, "A名稱");
        assert_eq!(semi.top_sellers.len(), 1);
        assert_eq!(semi.top_sellers[0].net_amount, -50_000.0);

        let shipping = &response.items[1];
        assert_eq!(shipping.net_amount, -300_000.0);
        assert!(shipping.top_buyers.is_empty());
        let sellers: Vec<(&str, f64)> = shipping
            .top_sellers
            .iter()
            .map(|stock| (stock.stock_symbol.as_str(), stock.net_amount))
            .collect();
        assert_eq!(sellers, [("F", -200_000.0), ("G", -100_000.0)]);
    }

    /// 市場篩選、asc 反轉（名次不變）、沒有成交金額時占比為 null。
    #[test]
    fn build_response_filters_market_and_reverses() {
        let mut rows = universe();
        for row in &mut rows {
            row.trade_value = None;
        }
        let response = build_response(period(), "foreign", "tpex", 4, "asc", rows);
        assert_eq!(response.market, "tpex");
        assert_eq!(response.total_net_amount, -150_000.0);
        let names: Vec<(usize, &str, f64)> = response
            .items
            .iter()
            .map(|item| (item.rank, item.industry.as_str(), item.net_amount))
            .collect();
        assert_eq!(
            names,
            [(2, "航運業", -100_000.0), (1, "半導體業", -50_000.0)]
        );
        assert!(response.items[0].trade_value_ratio.is_none());
    }
}
