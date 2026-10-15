//! `/market/margin-ranking` handler：融資融券增減與券資比排行。
//!
//! 資料來自 `chip_daily` 的融資融券餘額（張，每晚 21:50 入庫）。期間增減＝資料日餘額減期間第一天的
//! 前日餘額；券資比＝融券餘額 ÷ 融資餘額。只列資料日有融資融券資料（可信用交易）的上市櫃股票。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Local;

use super::institutional_flow::{FlowPeriod, MAX_FLOW_DAYS, chip_dates, flow_period};
use super::relative_strength::market_label;
use super::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    round2,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarginRankItem, MarginRankingParams, MarginRankingResponse,
};

/// 合法的排序鍵。
const RANK_BY: [&str; 5] = [
    "margin_increase",
    "margin_decrease",
    "short_increase",
    "short_decrease",
    "short_margin_ratio",
];

/// 券資比排行要求的最低融資餘額（張），避免融資極少的股票因分母小而排在前面。
const MIN_MARGIN_FOR_RATIO: i64 = 100;

/// 上市櫃股票在 `$2`～`$1` 期間的融資融券期初（期間第一天的前日餘額）與資料日餘額。
const MARGIN_SQL: &str = r#"
WITH period AS (
    SELECT stock_symbol,
           MAX("date") AS last_date,
           (ARRAY_AGG(margin_previous ORDER BY "date"))[1] AS margin_start,
           (ARRAY_AGG(margin_balance ORDER BY "date" DESC))[1] AS margin_balance,
           (ARRAY_AGG(short_previous ORDER BY "date"))[1] AS short_start,
           (ARRAY_AGG(short_balance ORDER BY "date" DESC))[1] AS short_balance
    FROM chip_daily
    WHERE "date" BETWEEN $2 AND $1 AND margin_balance IS NOT NULL
    GROUP BY stock_symbol
)
SELECT p.stock_symbol,
       s."Name" AS name,
       s.stock_exchange_market_id AS market_id,
       COALESCE(si.name, '') AS industry,
       COALESCE(p.margin_start, 0) AS margin_start,
       p.margin_balance,
       COALESCE(p.short_start, 0) AS short_start,
       COALESCE(p.short_balance, 0) AS short_balance,
       q."ClosingPrice"::float8 AS closing_price
FROM period p
JOIN stocks s ON s.stock_symbol = p.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN "DailyQuotes" q
       ON q.stock_symbol = p.stock_symbol AND q."Date" = $1 AND q."ClosingPrice" > 0
WHERE p.last_date = $1
  AND s.stock_exchange_market_id IN (2, 4)
"#;

/// [`MARGIN_SQL`] 的一列；餘額單位為張。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct MarginRow {
    stock_symbol: String,
    name: String,
    market_id: i32,
    industry: String,
    margin_start: i64,
    margin_balance: i64,
    short_start: i64,
    short_balance: i64,
    closing_price: Option<f64>,
}

/// 查詢融資融券排行。
///
/// 比較最近 `days` 個有籌碼資料的交易日：增減排行只列方向相符的股票（例如融資增加只列增加者），
/// 券資比排行只列融資餘額至少 100 張的股票。
///
/// # Errors
///
/// 參數格式錯誤、`days` 超出 1–60、`limit` 超出 1–100、`rank_by`／`market` 不合法或產業名稱不存在
/// 回 422；查無籌碼資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/margin-ranking", tag = "data-api", params(MarginRankingParams), responses((status = 200, body = MarginRankingResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn margin_ranking(Query(params): Query<MarginRankingParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(1);
    if !(1..=MAX_FLOW_DAYS).contains(&usize::from(days)) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }
    let rank_by = params.rank_by.as_deref().unwrap_or("margin_increase");
    if !RANK_BY.contains(&rank_by) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rank_by 必須為 margin_increase、margin_decrease、short_increase、short_decrease 或 short_margin_ratio",
        );
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
    let dates = match chip_dates(until).await {
        Ok(dates) => dates,
        Err(response) => return *response,
    };
    let Some(period) = flow_period(&dates, usize::from(days)) else {
        return error_response(StatusCode::NOT_FOUND, "查無籌碼資料");
    };
    let rows: Vec<MarginRow> = match sqlx::query_as(MARGIN_SQL)
        .bind(period.data_date)
        .bind(period.from)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    Json(build_response(
        period,
        RankFilter {
            rank_by,
            market,
            market_id,
            industry: params.industry.as_deref(),
            limit: usize::from(limit),
        },
        rows,
    ))
    .into_response()
}

/// 排行的篩選與排序條件。
struct RankFilter<'a> {
    rank_by: &'a str,
    market: &'a str,
    market_id: i32,
    industry: Option<&'a str>,
    limit: usize,
}

/// 依排序鍵取得一列的排序值；不符合該排行（方向不對或融資太少）回 `None`。
/// 數值越大排越前面（減少類以負值的相反數排序）。
fn rank_value(rank_by: &str, item: &MarginRankItem) -> Option<f64> {
    match rank_by {
        "margin_increase" => (item.margin_change > 0).then_some(item.margin_change as f64),
        "margin_decrease" => (item.margin_change < 0).then_some(-item.margin_change as f64),
        "short_increase" => (item.short_change > 0).then_some(item.short_change as f64),
        "short_decrease" => (item.short_change < 0).then_some(-item.short_change as f64),
        _ => item
            .short_margin_ratio
            .filter(|_| item.margin_balance >= MIN_MARGIN_FOR_RATIO),
    }
}

/// 一列轉成排行項目（名次稍後填入）。
fn rank_item(row: MarginRow) -> MarginRankItem {
    let margin_change = row.margin_balance - row.margin_start;
    MarginRankItem {
        rank: 0,
        market: market_label(row.market_id).to_owned(),
        margin_change,
        margin_change_percent: (row.margin_start > 0)
            .then(|| round2(margin_change as f64 / row.margin_start as f64 * 100.0)),
        short_change: row.short_balance - row.short_start,
        short_margin_ratio: (row.margin_balance > 0)
            .then(|| round2(row.short_balance as f64 / row.margin_balance as f64 * 100.0)),
        margin_balance: row.margin_balance,
        short_balance: row.short_balance,
        closing_price: row.closing_price,
        stock_symbol: row.stock_symbol,
        name: row.name,
        industry: row.industry,
    }
}

/// 依市場、產業篩選，依排序鍵由大到小排列（同值依代號），取前 `limit` 筆。
fn build_response(
    period: FlowPeriod,
    filter: RankFilter<'_>,
    rows: Vec<MarginRow>,
) -> MarginRankingResponse {
    let mut ranked: Vec<(f64, MarginRankItem)> = rows
        .into_iter()
        .filter(|row| {
            (filter.market_id == 0 || row.market_id == filter.market_id)
                && filter
                    .industry
                    .is_none_or(|industry| row.industry == industry)
        })
        .map(rank_item)
        .filter_map(|item| Some((rank_value(filter.rank_by, &item)?, item)))
        .collect();
    ranked.sort_by(|(a, item_a), (b, item_b)| {
        b.total_cmp(a)
            .then_with(|| item_a.stock_symbol.cmp(&item_b.stock_symbol))
    });
    let matched = ranked.len();
    let items = ranked
        .into_iter()
        .take(filter.limit)
        .enumerate()
        .map(|(index, (_, mut item))| {
            item.rank = index + 1;
            item
        })
        .collect();

    MarginRankingResponse {
        data_as_of: period.data_date.to_string(),
        from: period.from.to_string(),
        days: period.days,
        rank_by: filter.rank_by.to_owned(),
        market: filter.market.to_owned(),
        industry: filter.industry.map(str::to_owned),
        matched,
        items,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn test_period() -> FlowPeriod {
        FlowPeriod {
            data_date: NaiveDate::from_ymd_opt(2026, 10, 14).expect("合法日期"),
            from: NaiveDate::from_ymd_opt(2026, 10, 8).expect("合法日期"),
            days: 5,
        }
    }

    fn row(
        symbol: &str,
        market_id: i32,
        industry: &str,
        margin: [i64; 2],
        short: [i64; 2],
    ) -> MarginRow {
        MarginRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol}名稱"),
            market_id,
            industry: industry.to_owned(),
            margin_start: margin[0],
            margin_balance: margin[1],
            short_start: short[0],
            short_balance: short[1],
            closing_price: Some(20.0),
        }
    }

    fn universe() -> Vec<MarginRow> {
        vec![
            row("A", 2, "半導體業", [1_000, 1_500], [100, 50]),
            row("B", 4, "半導體業", [2_000, 1_200], [10, 300]),
            row("C", 2, "航運業", [0, 80], [0, 40]),
            row("D", 2, "航運業", [500, 1_000], [200, 260]),
            row("E", 4, "", [300, 300], [30, 30]),
        ]
    }

    fn filter<'a>(rank_by: &'a str, market_id: i32, industry: Option<&'a str>) -> RankFilter<'a> {
        RankFilter {
            rank_by,
            market: if market_id == 4 { "tpex" } else { "all" },
            market_id,
            industry,
            limit: 2,
        }
    }

    fn symbols(response: &MarginRankingResponse) -> Vec<(usize, &str)> {
        response
            .items
            .iter()
            .map(|item| (item.rank, item.stock_symbol.as_str()))
            .collect()
    }

    /// 融資增加：同增減依代號、limit 截斷但 matched 不變；增減、百分比、券資比的換算。
    #[test]
    fn build_response_ranks_margin_increase() {
        let response = build_response(
            test_period(),
            filter("margin_increase", 0, None),
            universe(),
        );
        assert_eq!(response.data_as_of, "2026-10-14");
        assert_eq!(response.from, "2026-10-08");
        assert_eq!(response.days, 5);
        assert_eq!(response.rank_by, "margin_increase");
        assert_eq!(response.matched, 3, "B 減少、E 不變不列入");
        assert_eq!(symbols(&response), [(1, "A"), (2, "D")]);
        let a = &response.items[0];
        assert_eq!((a.margin_balance, a.margin_change), (1_500, 500));
        assert_eq!(a.margin_change_percent, Some(50.0));
        assert_eq!((a.short_balance, a.short_change), (50, -50));
        assert_eq!(a.short_margin_ratio, Some(3.33));
        assert_eq!(a.market, "twse");
        assert_eq!(a.name, "A名稱");
        assert_eq!(a.closing_price, Some(20.0));
    }

    /// 其餘四種排序、期初為 0 的百分比為 null、券資比需融資至少 100 張、市場與產業篩選。
    #[test]
    fn build_response_other_rankings_and_filters() {
        let decrease = build_response(
            test_period(),
            filter("margin_decrease", 0, None),
            universe(),
        );
        assert_eq!(symbols(&decrease), [(1, "B")]);
        assert_eq!(decrease.items[0].margin_change, -800);
        assert_eq!(decrease.items[0].margin_change_percent, Some(-40.0));

        let short_up = build_response(test_period(), filter("short_increase", 0, None), universe());
        assert_eq!(symbols(&short_up), [(1, "B"), (2, "D")]);
        let short_down =
            build_response(test_period(), filter("short_decrease", 0, None), universe());
        assert_eq!(symbols(&short_down), [(1, "A")]);

        // C 券資比 50% 最高，但融資只有 80 張不列入；D 26% 高於 B 25%。
        let ratio = build_response(
            test_period(),
            filter("short_margin_ratio", 0, None),
            universe(),
        );
        assert_eq!(symbols(&ratio), [(1, "D"), (2, "B")]);
        assert_eq!(ratio.items[0].short_margin_ratio, Some(26.0));
        assert_eq!(ratio.matched, 4);

        let new_margin = build_response(
            test_period(),
            filter("margin_increase", 0, Some("航運業")),
            universe(),
        );
        assert_eq!(new_margin.industry.as_deref(), Some("航運業"));
        assert_eq!(symbols(&new_margin), [(1, "D"), (2, "C")]);
        assert_eq!(new_margin.items[1].margin_change_percent, None);

        let otc = build_response(test_period(), filter("short_increase", 4, None), universe());
        assert_eq!(otc.market, "tpex");
        assert_eq!(symbols(&otc), [(1, "B")]);
    }
}
