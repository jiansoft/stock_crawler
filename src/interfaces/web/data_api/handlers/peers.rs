//! `/stocks/{symbol}/peers` handler：同產業個股的估值、殖利率、財務體質與營收成長比較。
//!
//! 報價與倍數取 `last_daily_quotes`（近 30 天有成交），殖利率取 `yield_rank`（近 30 天），
//! F 分數與毛利率取 `financial_health`（新鮮度與選股相同），營收年增率取最近 3 個月內
//! 最新一個月。統計規則在 [`crate::domain::quote::peer`]。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, Local, Months, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy, prelude::ToPrimitive};

use super::{database_error, error_response};
use crate::domain::quote::peer::{self, Better};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PeerMetric, PeerStock, PeersParams, PeersResponse,
};

/// 目標股的產業；同時用來判斷股票是否存在。
const TARGET_SQL: &str = r#"
SELECT s.stock_industry_id, COALESCE(si.name, '')
FROM stocks s
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
WHERE s.stock_symbol = $1
"#;

/// 同產業近 30 天有成交的個股（目標股不受市場、下市與 ETF 條件限制）。
/// 各來源的「最新一筆」走以股票代號為前導欄位的索引。
const PEERS_SQL: &str = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s.stock_exchange_market_id AS market_id,
    q.date AS quote_date,
    q.closing_price,
    CASE WHEN s.issued_share > 0 THEN q.closing_price * s.issued_share END AS market_cap,
    q.price_earning_ratio,
    q."price-to-book_ratio" AS price_to_book_ratio,
    y.yield AS dividend_yield_percent,
    CASE WHEN EXTRACT(YEAR FROM $3::date)::int - h.f_score_year BETWEEN 1 AND 2
      THEN h.f_score END AS f_score,
    CASE WHEN ((EXTRACT(YEAR FROM $3::date)::int * 4 + EXTRACT(QUARTER FROM $3::date)::int)
          - (h.quarter_year * 4 + h.quarter)) BETWEEN 0 AND 2
      THEN h.gross_margin_percent END AS gross_margin_percent,
    r.yoy AS revenue_yoy_percent
FROM stocks s
JOIN last_daily_quotes q
  ON q.stock_symbol = s.stock_symbol
 AND q.closing_price > 0
 AND q.date >= $3::date - 30
LEFT JOIN LATERAL (
    SELECT yield FROM yield_rank
    WHERE security_code = s.stock_symbol AND date >= $3::date - 30
    ORDER BY date DESC
    LIMIT 1
) y ON TRUE
LEFT JOIN financial_health h ON h.stock_symbol = s.stock_symbol
LEFT JOIN LATERAL (
    SELECT "ComparedWithLastYearSameMonth" AS yoy FROM "Revenue"
    WHERE "SecurityCode" = s.stock_symbol AND "Date" >= $4
    ORDER BY "Date" DESC
    LIMIT 1
) r ON TRUE
WHERE s.stock_industry_id = $1
  AND (s.stock_symbol = $2
    OR (s.stock_exchange_market_id IN (2, 4)
        AND NOT s."SuspendListing"
        AND s.stock_symbol !~ '^00'))
"#;

/// 同業一列。
#[derive(Debug, Clone, sqlx::FromRow)]
struct PeerRow {
    stock_symbol: String,
    name: String,
    market_id: i32,
    quote_date: NaiveDate,
    closing_price: Decimal,
    market_cap: Option<Decimal>,
    price_earning_ratio: Decimal,
    price_to_book_ratio: Decimal,
    dividend_yield_percent: Option<Decimal>,
    f_score: Option<i16>,
    gross_margin_percent: Option<Decimal>,
    revenue_yoy_percent: Option<Decimal>,
}

/// 比較的指標：名稱、取值、方向、是否只算正值。
type MetricSpec = (&'static str, fn(&PeerRow) -> Option<Decimal>, Better, bool);

const METRICS: [MetricSpec; 7] = [
    ("market_cap", |row| row.market_cap, Better::Higher, false),
    (
        "price_earning_ratio",
        |row| Some(row.price_earning_ratio),
        Better::Lower,
        true,
    ),
    (
        "price_to_book_ratio",
        |row| Some(row.price_to_book_ratio),
        Better::Lower,
        true,
    ),
    (
        "dividend_yield_percent",
        |row| row.dividend_yield_percent,
        Better::Higher,
        false,
    ),
    (
        "f_score",
        |row| row.f_score.map(Decimal::from),
        Better::Higher,
        false,
    ),
    (
        "gross_margin_percent",
        |row| row.gross_margin_percent,
        Better::Higher,
        false,
    ),
    (
        "revenue_yoy_percent",
        |row| row.revenue_yoy_percent,
        Better::Higher,
        false,
    ),
];

/// 查詢單一股票與同產業個股的比較。
///
/// # Errors
///
/// `limit` 不在 1–100 回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/peers", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), PeersParams), responses((status = 200, body = PeersResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_peers(
    Path(symbol): Path<String>,
    Query(params): Query<PeersParams>,
) -> Response {
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 100");
    }
    let pool = database::get_connection();
    let target: Option<(i32, String)> = match sqlx::query_as(TARGET_SQL)
        .bind(&symbol)
        .fetch_optional(pool)
        .await
    {
        Ok(target) => target,
        Err(error) => return database_error(error),
    };
    let Some((industry_id, industry_name)) = target else {
        return error_response(StatusCode::NOT_FOUND, "找不到股票");
    };

    let today = Local::now().date_naive();
    let rows: Vec<PeerRow> = match sqlx::query_as(PEERS_SQL)
        .bind(industry_id)
        .bind(&symbol)
        .bind(today)
        .bind(revenue_month_floor(today))
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    Json(build_response(
        symbol,
        industry_id,
        industry_name,
        usize::from(limit),
        rows,
    ))
    .into_response()
}

/// 營收資料最舊可接受的月份（`YYYYMM`）：今天往前 3 個月。
fn revenue_month_floor(today: NaiveDate) -> i64 {
    let floor = today.checked_sub_months(Months::new(3)).unwrap_or(today);
    i64::from(floor.year()) * 100 + i64::from(floor.month())
}

/// 計算各指標的中位數與目標名次，並依市值由大到小列出前 `limit` 檔。
fn build_response(
    symbol: String,
    industry_id: i32,
    industry_name: String,
    limit: usize,
    mut rows: Vec<PeerRow>,
) -> PeersResponse {
    let target = rows.iter().find(|row| row.stock_symbol == symbol).cloned();
    let metrics = METRICS
        .iter()
        .map(|(name, pick, better, positive_only)| {
            let values: Vec<Option<Decimal>> = rows.iter().map(pick).collect();
            let target_value = target.as_ref().and_then(pick);
            let standing = peer::standing(target_value, &values, *better, *positive_only);
            PeerMetric {
                metric: (*name).to_owned(),
                better: better.as_str().to_owned(),
                target: target_value
                    .filter(|value| !positive_only || *value > Decimal::ZERO)
                    .and_then(round2),
                industry_median: standing.median.and_then(round2),
                rank: standing.rank,
                samples: standing.samples,
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        b.market_cap
            .cmp(&a.market_cap)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    PeersResponse {
        data_as_of: rows
            .iter()
            .map(|row| row.quote_date)
            .max()
            .map(|date| date.to_string()),
        peer_count: rows.len(),
        target: target.map(to_peer_stock),
        metrics,
        peers: rows.into_iter().take(limit).map(to_peer_stock).collect(),
        stock_symbol: symbol,
        industry_id,
        industry_name,
    }
}

/// 資料庫列轉成 API 契約；本益比、股價淨值比 ≤ 0 視為沒有值，數值一律四捨五入到小數 2 位。
fn to_peer_stock(row: PeerRow) -> PeerStock {
    let positive = |value: Decimal| (value > Decimal::ZERO).then(|| round2(value)).flatten();
    PeerStock {
        closing_price: round2(row.closing_price),
        market_cap: row.market_cap.and_then(round2),
        price_earning_ratio: positive(row.price_earning_ratio),
        price_to_book_ratio: positive(row.price_to_book_ratio),
        dividend_yield_percent: row.dividend_yield_percent.and_then(round2),
        f_score: row.f_score,
        gross_margin_percent: row.gross_margin_percent.and_then(round2),
        revenue_yoy_percent: row.revenue_yoy_percent.and_then(round2),
        stock_symbol: row.stock_symbol,
        name: row.name,
        market_id: row.market_id,
    }
}

/// 四捨五入到小數 2 位的 JSON number（資料庫欄位多為 4 位小數，例如淨值比 10.2399）。
fn round2(value: Decimal) -> Option<f64> {
    value
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .to_f64()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn row(
        symbol: &str,
        market_cap: Option<Decimal>,
        pe: Decimal,
        yoy: Option<Decimal>,
    ) -> PeerRow {
        PeerRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol} 名稱"),
            market_id: 2,
            quote_date: NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期"),
            closing_price: dec!(100),
            market_cap,
            price_earning_ratio: pe,
            price_to_book_ratio: dec!(2),
            dividend_yield_percent: None,
            f_score: Some(6),
            gross_margin_percent: Some(dec!(30)),
            revenue_yoy_percent: yoy,
        }
    }

    fn metric<'a>(response: &'a PeersResponse, name: &str) -> &'a PeerMetric {
        response
            .metrics
            .iter()
            .find(|metric| metric.metric == name)
            .expect("應有此指標")
    }

    /// 個股欄位、指標目標值與中位數都四捨五入到小數 2 位。
    #[test]
    fn values_are_rounded_to_two_decimals() {
        let mut target = row("2330", Some(dec!(65868219970180.0001)), dec!(29.444), None);
        target.price_to_book_ratio = dec!(10.2399);
        target.dividend_yield_percent = Some(dec!(0.2756));
        target.gross_margin_percent = Some(dec!(67.7207));
        target.revenue_yoy_percent = Some(dec!(54.645));
        let mut other = row("2454", Some(dec!(100)), dec!(76.3849), None);
        other.price_to_book_ratio = dec!(17.4023);

        let response = build_response(
            "2330".to_owned(),
            24,
            "半導體業".to_owned(),
            20,
            vec![target, other],
        );
        let stock = response.target.as_ref().expect("有目標股");
        assert_eq!(stock.market_cap, Some(65_868_219_970_180.0));
        assert_eq!(stock.price_earning_ratio, Some(29.44));
        assert_eq!(stock.price_to_book_ratio, Some(10.24));
        assert_eq!(stock.dividend_yield_percent, Some(0.28));
        assert_eq!(stock.gross_margin_percent, Some(67.72));
        assert_eq!(
            stock.revenue_yoy_percent,
            Some(54.65),
            "四捨五入：54.645 → 54.65"
        );
        let pb = metric(&response, "price_to_book_ratio");
        assert_eq!((pb.target, pb.industry_median), (Some(10.24), Some(13.82)));
    }

    /// 中位數與名次用全部同業；peers 依市值由大到小只列前 limit 檔，沒有市值的排最後。
    #[test]
    fn build_response_ranks_target_among_all_peers() {
        let rows = vec![
            row("1101", Some(dec!(300)), dec!(10), Some(dec!(5))),
            row("1102", Some(dec!(100)), dec!(20), Some(dec!(-3))),
            row("1103", None, dec!(0), None),
            row("1104", Some(dec!(200)), dec!(30), Some(dec!(12))),
        ];
        let response = build_response("1102".to_owned(), 1, "水泥工業".to_owned(), 2, rows);

        assert_eq!(response.peer_count, 4);
        assert_eq!(response.data_as_of.as_deref(), Some("2026-10-12"));
        assert_eq!(
            response
                .peers
                .iter()
                .map(|peer| peer.stock_symbol.as_str())
                .collect::<Vec<_>>(),
            ["1101", "1104"]
        );
        let target = response.target.as_ref().expect("有目標股");
        assert_eq!(target.stock_symbol, "1102");

        let pe = metric(&response, "price_earning_ratio");
        assert_eq!(pe.better, "lower");
        assert_eq!((pe.target, pe.industry_median), (Some(20.0), Some(20.0)));
        assert_eq!((pe.rank, pe.samples), (Some(2), 3));

        let cap = metric(&response, "market_cap");
        assert_eq!((cap.rank, cap.samples), (Some(3), 3));
        let yoy = metric(&response, "revenue_yoy_percent");
        assert_eq!((yoy.rank, yoy.industry_median), (Some(3), Some(5.0)));
        let f_score = metric(&response, "f_score");
        assert_eq!((f_score.rank, f_score.samples), (Some(1), 4));
    }

    /// 目標股近期沒有報價：target 為 null、名次為 null，中位數照算；虧損股本益比輸出 null。
    #[test]
    fn build_response_without_target_quote() {
        let rows = vec![row("1101", Some(dec!(300)), dec!(0), None)];
        let response = build_response("0050".to_owned(), 0, String::new(), 20, rows);
        assert!(response.target.is_none());
        assert!(response.metrics.iter().all(|metric| metric.rank.is_none()));
        assert_eq!(response.peers[0].price_earning_ratio, None);
        assert_eq!(
            metric(&response, "price_to_book_ratio").industry_median,
            Some(2.0)
        );
    }

    /// 營收最舊可接受月份為今天往前 3 個月（跨年）。
    #[test]
    fn revenue_month_floor_goes_back_three_months() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("有效日期");
        assert_eq!(revenue_month_floor(day(2026, 10, 12)), 202607);
        assert_eq!(revenue_month_floor(day(2026, 2, 28)), 202511);
    }
}
