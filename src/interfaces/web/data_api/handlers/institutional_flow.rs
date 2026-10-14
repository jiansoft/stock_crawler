//! `/market/institutional-flow` handler：三大法人買賣超排行。
//!
//! 資料來自 `chip_daily`（每晚 21:50 入庫的全市場三大法人買賣超股數）。期間累計淨股數，
//! 估計金額＝每日淨股數 × 當日收盤價加總，依金額排名；另附占期間成交量比例與截至資料日的
//! 連續買賣超天數（最多回看 [`STREAK_WINDOW`] 個有籌碼資料的交易日）。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::relative_strength::market_label;
use super::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    round2,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, InstitutionalFlowItem, InstitutionalFlowParams, InstitutionalFlowResponse,
};

/// 連續買賣超天數最多回看的交易日數，也是 `days` 的上限。
pub(super) const STREAK_WINDOW: usize = 60;

/// 合法的法人別。
pub(super) const INVESTORS: [&str; 4] = ["foreign", "trust", "dealer", "total"];

/// `$1`（含）之前最近 [`STREAK_WINDOW`] 個有籌碼資料的交易日，由新到舊。
///
/// 只回看 200 個日曆天，足以涵蓋 60 個交易日（含春節等長假）。
const DATES_SQL: &str = r#"
SELECT DISTINCT "date"
FROM chip_daily
WHERE "date" <= $1 AND "date" > $1 - 200
ORDER BY 1 DESC
LIMIT $2
"#;

/// 上市櫃股票在 `$2`～`$1` 期間的法人（`$4`）淨買賣，與截至 `$1` 的連續天數（回看到 `$3`）。
///
/// - `total` 為三者相加，任一為 NULL（來源沒有這檔）即為 NULL。
/// - 連續天數：資料日的買賣方向往回數，遇到方向不同或沒有買賣的一天即停；資料日沒有買賣為 0。
/// - 金額以當日收盤價估算；沒有成交的日子不計金額與成交量。
const FLOW_SQL: &str = r#"
WITH net AS (
    SELECT stock_symbol, "date",
           CASE $4
               WHEN 'foreign' THEN foreign_net
               WHEN 'trust' THEN trust_net
               WHEN 'dealer' THEN dealer_net
               ELSE foreign_net + trust_net + dealer_net
           END AS net
    FROM chip_daily
    WHERE "date" BETWEEN $3 AND $1
),
latest AS (
    SELECT stock_symbol, SIGN(net)::int AS sign
    FROM net
    WHERE "date" = $1 AND net <> 0
),
breaks AS (
    SELECT l.stock_symbol, l.sign,
           MAX(n."date") FILTER (WHERE n.net IS NULL OR SIGN(n.net)::int <> l.sign) AS break_date
    FROM latest l
    JOIN net n ON n.stock_symbol = l.stock_symbol
    GROUP BY l.stock_symbol, l.sign
),
streaks AS (
    SELECT b.stock_symbol, (b.sign * COUNT(*))::int AS streak
    FROM breaks b
    JOIN net n ON n.stock_symbol = b.stock_symbol
              AND n."date" > COALESCE(b.break_date, '-infinity'::date)
    GROUP BY b.stock_symbol, b.sign
),
period AS (
    SELECT n.stock_symbol,
           SUM(n.net)::bigint AS net_shares,
           COALESCE(SUM(n.net * q."ClosingPrice"), 0)::float8 AS net_amount,
           SUM(q."TradingVolume")::float8 AS volume,
           COUNT(*) FILTER (WHERE n.net > 0) AS buy_days,
           COUNT(*) FILTER (WHERE n.net < 0) AS sell_days
    FROM net n
    LEFT JOIN "DailyQuotes" q
           ON q.stock_symbol = n.stock_symbol AND q."Date" = n."date" AND q."ClosingPrice" > 0
    WHERE n."date" >= $2 AND n.net IS NOT NULL
    GROUP BY n.stock_symbol
)
SELECT p.stock_symbol,
       s."Name" AS name,
       s.stock_exchange_market_id AS market_id,
       COALESCE(si.name, '') AS industry,
       p.net_shares,
       p.net_amount,
       p.volume,
       p.buy_days,
       p.sell_days,
       COALESCE(st.streak, 0) AS streak,
       dq."ClosingPrice"::float8 AS closing_price
FROM period p
JOIN stocks s ON s.stock_symbol = p.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN streaks st ON st.stock_symbol = p.stock_symbol
LEFT JOIN "DailyQuotes" dq
       ON dq.stock_symbol = p.stock_symbol AND dq."Date" = $1 AND dq."ClosingPrice" > 0
WHERE s.stock_exchange_market_id IN (2, 4)
"#;

/// [`FLOW_SQL`] 的一列：一檔股票在期間內的法人淨買賣。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(super) struct FlowRow {
    pub(super) stock_symbol: String,
    pub(super) name: String,
    pub(super) market_id: i32,
    pub(super) industry: String,
    pub(super) net_shares: i64,
    pub(super) net_amount: f64,
    pub(super) volume: Option<f64>,
    pub(super) buy_days: i64,
    pub(super) sell_days: i64,
    pub(super) streak: i32,
    pub(super) closing_price: Option<f64>,
}

/// 累計期間：資料日、期間第一天與實際天數。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FlowPeriod {
    pub(super) data_date: NaiveDate,
    pub(super) from: NaiveDate,
    pub(super) days: usize,
}

/// 由新到舊的籌碼交易日決定累計期間與連續天數的回看起點；沒有交易日回 `None`。
fn flow_period(dates: &[NaiveDate], days: usize) -> Option<(FlowPeriod, NaiveDate)> {
    let data_date = *dates.first()?;
    let days = days.min(dates.len());
    let window_start = *dates.last()?;
    Some((
        FlowPeriod {
            data_date,
            from: dates[days - 1],
            days,
        },
        window_start,
    ))
}

/// 載入 `until`（含）之前最近 `days` 個籌碼交易日、法人別 `investor` 的全體上市櫃淨買賣。
///
/// 沒有任何籌碼資料回 `Ok(None)`；查詢失敗時回已組好的安全 500 回應，呼叫端直接回傳即可。
pub(super) async fn load_flows(
    until: NaiveDate,
    days: usize,
    investor: &str,
) -> Result<Option<(FlowPeriod, Vec<FlowRow>)>, Box<Response>> {
    let dates: Vec<NaiveDate> = sqlx::query_scalar(DATES_SQL)
        .bind(until)
        .bind(STREAK_WINDOW as i64)
        .fetch_all(database::get_connection())
        .await
        .map_err(|error| Box::new(database_error(error)))?;
    let Some((period, window_start)) = flow_period(&dates, days) else {
        return Ok(None);
    };
    let rows: Vec<FlowRow> = sqlx::query_as(FLOW_SQL)
        .bind(period.data_date)
        .bind(period.from)
        .bind(window_start)
        .bind(investor)
        .fetch_all(database::get_connection())
        .await
        .map_err(|error| Box::new(database_error(error)))?;
    Ok(Some((period, rows)))
}

/// 查詢三大法人買賣超排行。
///
/// 累計最近 `days` 個有籌碼資料的交易日，依估計淨買賣金額（每日淨股數 × 當日收盤價）排名；
/// 籌碼資料每晚 21:50 入庫，當天晚上之前的最新資料日是前一個交易日。
///
/// # Errors
///
/// 參數格式錯誤、`days` 超出 1–60、`limit` 超出 1–100、`investor`／`side`／`market` 不合法或
/// 產業名稱不存在回 422；查無籌碼資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/institutional-flow", tag = "data-api", params(InstitutionalFlowParams), responses((status = 200, body = InstitutionalFlowResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn institutional_flow(Query(params): Query<InstitutionalFlowParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(1);
    if !(1..=STREAK_WINDOW).contains(&usize::from(days)) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }
    let investor = params.investor.as_deref().unwrap_or("foreign");
    if !INVESTORS.contains(&investor) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "investor 必須為 foreign、trust、dealer 或 total",
        );
    }
    let side = params.side.as_deref().unwrap_or("buy");
    if !matches!(side, "buy" | "sell") {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "side 必須為 buy 或 sell");
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let limit = params.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 100");
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }

    let until = date.unwrap_or_else(|| Local::now().date_naive());
    match load_flows(until, usize::from(days), investor).await {
        Ok(Some((period, rows))) => Json(build_response(
            period,
            FlowFilter {
                investor,
                side,
                market,
                market_id,
                industry: params.industry.as_deref(),
                limit: usize::from(limit),
            },
            rows,
        ))
        .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無籌碼資料"),
        Err(response) => *response,
    }
}

/// 排行的篩選與排序條件。
struct FlowFilter<'a> {
    investor: &'a str,
    side: &'a str,
    market: &'a str,
    market_id: i32,
    industry: Option<&'a str>,
    limit: usize,
}

/// 依市場、產業與方向篩選，買超依金額由大到小、賣超由小（負最多）到大，同金額依代號排列。
fn build_response(
    period: FlowPeriod,
    filter: FlowFilter<'_>,
    rows: Vec<FlowRow>,
) -> InstitutionalFlowResponse {
    let buy = filter.side == "buy";
    let mut rows: Vec<FlowRow> = rows
        .into_iter()
        .filter(|row| {
            (filter.market_id == 0 || row.market_id == filter.market_id)
                && filter
                    .industry
                    .is_none_or(|industry| row.industry == industry)
                && if buy {
                    row.net_amount > 0.0
                } else {
                    row.net_amount < 0.0
                }
        })
        .collect();
    rows.sort_by(|a, b| {
        let by_amount = if buy {
            b.net_amount.total_cmp(&a.net_amount)
        } else {
            a.net_amount.total_cmp(&b.net_amount)
        };
        by_amount.then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    let matched = rows.len();
    let items = rows
        .into_iter()
        .take(filter.limit)
        .enumerate()
        .map(|(index, row)| flow_item(index + 1, row))
        .collect();

    InstitutionalFlowResponse {
        data_as_of: period.data_date.to_string(),
        from: period.from.to_string(),
        days: period.days,
        investor: filter.investor.to_owned(),
        side: filter.side.to_owned(),
        market: filter.market.to_owned(),
        industry: filter.industry.map(str::to_owned),
        matched,
        items,
    }
}

/// 一列轉成排行項目：張數四捨五入、金額取整數元、成交量占比取兩位小數。
fn flow_item(rank: usize, row: FlowRow) -> InstitutionalFlowItem {
    InstitutionalFlowItem {
        rank,
        market: market_label(row.market_id).to_owned(),
        net_lots: (row.net_shares as f64 / 1000.0).round() as i64,
        net_amount: row.net_amount.round(),
        volume_ratio: row
            .volume
            .filter(|volume| *volume > 0.0)
            .map(|volume| round2(row.net_shares as f64 / volume * 100.0)),
        net_shares: row.net_shares,
        buy_days: row.buy_days,
        sell_days: row.sell_days,
        streak: row.streak,
        closing_price: row.closing_price,
        stock_symbol: row.stock_symbol,
        name: row.name,
        industry: row.industry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
    }

    fn row(symbol: &str, market_id: i32, industry: &str, net_shares: i64, amount: f64) -> FlowRow {
        FlowRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol}名稱"),
            market_id,
            industry: industry.to_owned(),
            net_shares,
            net_amount: amount,
            volume: Some(100_000.0),
            buy_days: 2,
            sell_days: 1,
            streak: 2,
            closing_price: Some(50.0),
        }
    }

    fn filter<'a>(side: &'a str, market_id: i32, industry: Option<&'a str>) -> FlowFilter<'a> {
        FlowFilter {
            investor: "trust",
            side,
            market: if market_id == 4 { "tpex" } else { "all" },
            market_id,
            industry,
            limit: 2,
        }
    }

    fn period() -> FlowPeriod {
        FlowPeriod {
            data_date: day(2026, 10, 13),
            from: day(2026, 10, 7),
            days: 3,
        }
    }

    /// 期間：資料日為第一天、天數不超過已有的交易日、回看起點為最舊一天；沒有交易日為 None。
    #[test]
    fn flow_period_counts_trading_days() {
        let dates = [day(2026, 10, 13), day(2026, 10, 12), day(2026, 10, 8)];
        let (period, window_start) = flow_period(&dates, 2).expect("有交易日");
        assert_eq!(period.data_date, day(2026, 10, 13));
        assert_eq!(period.from, day(2026, 10, 12));
        assert_eq!(period.days, 2);
        assert_eq!(window_start, day(2026, 10, 8));
        let (short, _) = flow_period(&dates, 60).expect("有交易日");
        assert_eq!((short.from, short.days), (day(2026, 10, 8), 3));
        assert!(flow_period(&[], 1).is_none());
    }

    fn universe() -> Vec<FlowRow> {
        vec![
            row("A", 2, "半導體業", 12_345, 600_000.4),
            row("B", 4, "半導體業", 8_000, 900_000.0),
            row("C", 2, "航運業", -20_600, -1_000_000.0),
            row("D", 2, "半導體業", 5_000, 600_000.4),
            row("E", 4, "航運業", -500, -20_000.0),
            row("F", 2, "航運業", 0, 0.0),
        ]
    }

    /// 買超依金額由大到小、同金額依代號；limit 截斷但 matched 不變；張數、金額與占比的換算。
    #[test]
    fn build_response_ranks_buyers() {
        let response = build_response(period(), filter("buy", 0, None), universe());
        assert_eq!(response.data_as_of, "2026-10-13");
        assert_eq!(response.from, "2026-10-07");
        assert_eq!(response.days, 3);
        assert_eq!(response.investor, "trust");
        assert_eq!(response.side, "buy");
        assert_eq!(response.market, "all");
        assert!(response.industry.is_none());
        assert_eq!(response.matched, 3, "金額為 0 的 F 不算買超");
        let ranked: Vec<(usize, &str)> = response
            .items
            .iter()
            .map(|item| (item.rank, item.stock_symbol.as_str()))
            .collect();
        assert_eq!(ranked, [(1, "B"), (2, "A")]);
        let a = &response.items[1];
        assert_eq!(a.market, "twse");
        assert_eq!(a.name, "A名稱");
        assert_eq!(a.net_shares, 12_345);
        assert_eq!(a.net_lots, 12);
        assert_eq!(a.net_amount, 600_000.0);
        assert_eq!(a.volume_ratio, Some(12.35));
        assert_eq!((a.buy_days, a.sell_days, a.streak), (2, 1, 2));
        assert_eq!(a.closing_price, Some(50.0));
        assert_eq!(response.items[0].market, "tpex");
    }

    /// 賣超依淨賣金額由大到小；市場與產業篩選；沒有成交量時占比為 null。
    #[test]
    fn build_response_ranks_sellers_with_filters() {
        let mut rows = universe();
        rows[2].volume = Some(0.0);
        let sellers = build_response(period(), filter("sell", 0, Some("航運業")), rows);
        assert_eq!(sellers.industry.as_deref(), Some("航運業"));
        let ranked: Vec<&str> = sellers
            .items
            .iter()
            .map(|item| item.stock_symbol.as_str())
            .collect();
        assert_eq!(ranked, ["C", "E"]);
        assert_eq!(sellers.items[0].net_lots, -21);
        assert_eq!(sellers.items[0].volume_ratio, None);

        let otc = build_response(period(), filter("sell", 4, None), universe());
        assert_eq!(otc.market, "tpex");
        assert_eq!(otc.matched, 1);
        assert_eq!(otc.items[0].stock_symbol, "E");
        assert_eq!(otc.items[0].volume_ratio, Some(-0.5));
    }
}
