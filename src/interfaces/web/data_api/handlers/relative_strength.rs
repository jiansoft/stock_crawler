//! `/market/relative-strength` handler：相對強度排行（RS）。
//!
//! 近 N 日報酬直接取 `daily_forward_return` 基準日 T−N 的 N 日欄位（見
//! [`crate::domain::quote::forward_return`]），不另外存表。加權報酬＝0.4 × 近 60 日
//! ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日（較重近一季），分數是加權報酬在上市櫃全體中的
//! 百分位（1–99）。市場、產業與代號只篩選輸出，分數與名次一律以全體計算。

use std::cmp::Ordering;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::{
    database_error, ensure_stock_exists, error_response, industry_error, market_id_for_stocks,
    parse_optional_date, repository_error, round2,
};
use crate::domain::quote::forward_return::{self, ForwardReturnRepository};
use crate::infra::database::{self, repository::quote::PgQuoteRepository};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, RelativeStrengthItem, RelativeStrengthParams, RelativeStrengthResponse,
};

/// 計分用的最長天期（交易日數）。
const LONGEST: usize = 240;

/// 加權報酬中近 60、120、240 日報酬的權重。
const WEIGHTS: [f64; 3] = [0.4, 0.3, 0.3];

/// 資料日 `$1` 的計分母體：上市櫃、未暫停交易，且近 60／120／240 日報酬都有的股票。
///
/// `$2`～`$5` 是往前第 20、60、120、240 個交易日（基準日）；近 N 日報酬＝該基準日的 N 日報酬。
const UNIVERSE_SQL: &str = r#"
SELECT
    r60.stock_symbol,
    s."Name" AS name,
    s.stock_exchange_market_id AS market_id,
    COALESCE(si.name, '') AS industry,
    r20.return_20d::float8 AS return_20d,
    r60.return_60d::float8 AS return_60d,
    r120.return_120d::float8 AS return_120d,
    r240.return_240d::float8 AS return_240d,
    q."ClosingPrice"::float8 AS closing_price
FROM daily_forward_return r60
JOIN daily_forward_return r120 ON r120.date = $4 AND r120.stock_symbol = r60.stock_symbol
JOIN daily_forward_return r240 ON r240.date = $5 AND r240.stock_symbol = r60.stock_symbol
LEFT JOIN daily_forward_return r20 ON r20.date = $2 AND r20.stock_symbol = r60.stock_symbol
JOIN stocks s ON s.stock_symbol = r60.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN "DailyQuotes" q
       ON q.stock_symbol = r60.stock_symbol AND q."Date" = $1 AND q."ClosingPrice" > 0
WHERE r60.date = $3
  AND r60.return_60d IS NOT NULL
  AND r120.return_120d IS NOT NULL
  AND r240.return_240d IS NOT NULL
  AND s.stock_exchange_market_id IN (2, 4)
  AND NOT s."SuspendListing"
"#;

/// 計分母體中的一檔股票。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(super) struct UniverseRow {
    pub(super) stock_symbol: String,
    pub(super) name: String,
    pub(super) market_id: i32,
    pub(super) industry: String,
    pub(super) return_20d: Option<f64>,
    pub(super) return_60d: f64,
    pub(super) return_120d: f64,
    pub(super) return_240d: f64,
    pub(super) closing_price: Option<f64>,
}

/// 已在全體中計分排名的一檔股票。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScoredStock {
    /// 名次（比它強的家數＋1，同分同名次）。
    pub(super) rank: usize,
    /// 相對強度分數 1–99。
    pub(super) rs_score: u8,
    /// 加權報酬（%）。
    pub(super) weighted: f64,
    pub(super) row: UniverseRow,
}

impl UniverseRow {
    /// 加權報酬（%）。
    fn weighted(&self) -> f64 {
        WEIGHTS[0] * self.return_60d + WEIGHTS[1] * self.return_120d + WEIGHTS[2] * self.return_240d
    }
}

/// 資料日與各天期的基準日。
#[derive(Debug, Clone, Copy, PartialEq)]
struct BaseDates {
    end: NaiveDate,
    d20: NaiveDate,
    d60: NaiveDate,
    d120: NaiveDate,
    d240: NaiveDate,
}

/// 以 `trading_dates`（由新到舊）的第一天為資料日，找出各天期的基準日；交易日不足回 `None`。
fn base_dates(trading_dates: &[NaiveDate]) -> Option<BaseDates> {
    let end = *trading_dates.first()?;
    let base = |horizon| forward_return::base_date(end, horizon, trading_dates);
    Some(BaseDates {
        end,
        d20: base(20)?,
        d60: base(60)?,
        d120: base(120)?,
        d240: base(LONGEST)?,
    })
}

/// 查詢上市櫃股票的相對強度分數與名次。
///
/// 分數以加權報酬（0.4 × 近 60 日 ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日，還原權值）在上市櫃
/// 全體中的百分位計算，99 為最強；上市未滿 240 個交易日的股票不計分。指定的交易日還沒算好
/// 報酬時（例如收盤匯總尚未跑完），改用前一個交易日。
///
/// # Errors
///
/// 參數格式錯誤、`order` 不是 asc／desc、`limit` 超出 1–200 或產業名稱不存在回 422；
/// `symbol` 不存在或查無資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/relative-strength", tag = "data-api", params(RelativeStrengthParams), responses((status = 200, body = RelativeStrengthResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn relative_strength(Query(params): Query<RelativeStrengthParams>) -> Response {
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
    if !matches!(params.order.as_deref(), None | Some("asc" | "desc")) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "order 必須為 asc 或 desc");
    }
    if !(1..=200).contains(&params.limit.unwrap_or(50)) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }
    if let Some(symbol) = params.symbol.as_deref()
        && let Some(response) = ensure_stock_exists(symbol).await
    {
        return response;
    }

    let until = date.unwrap_or_else(|| Local::now().date_naive());
    match load_scores(until).await {
        Ok(Some((data_date, scored))) => {
            Json(build_response(data_date, &params, market_id, scored)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無相對強度資料"),
        Err(response) => *response,
    }
}

/// 載入 `until`（含）之前最近一個已算好報酬的交易日的計分母體，並對全體計分排名。
///
/// 最新一個交易日的報酬可能還沒回填（例如收盤匯總尚未跑完），最多往前退一天；兩天都沒有
/// 資料回 `Ok(None)`。查詢失敗時回已組好的安全 500 回應，呼叫端直接回傳即可。
pub(super) async fn load_scores(
    until: NaiveDate,
) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>, Box<Response>> {
    let dates = PgQuoteRepository::new()
        .fetch_trading_dates(until, LONGEST + 2)
        .await
        .map_err(|error| Box::new(repository_error(error)))?;
    for offset in 0..2 {
        let Some(bases) = dates.get(offset..).and_then(base_dates) else {
            break;
        };
        let rows: Vec<UniverseRow> = sqlx::query_as(UNIVERSE_SQL)
            .bind(bases.end)
            .bind(bases.d20)
            .bind(bases.d60)
            .bind(bases.d120)
            .bind(bases.d240)
            .fetch_all(database::get_connection())
            .await
            .map_err(|error| Box::new(database_error(error)))?;
        if !rows.is_empty() {
            return Ok(Some((bases.end, score_universe(rows))));
        }
    }
    Ok(None)
}

/// 對全體計分與排名，由強到弱；同分同名次，並依代號排列讓輸出穩定。
pub(super) fn score_universe(rows: Vec<UniverseRow>) -> Vec<ScoredStock> {
    let universe = rows.len();
    let mut weighted: Vec<(f64, UniverseRow)> =
        rows.into_iter().map(|row| (row.weighted(), row)).collect();
    weighted.sort_by(|(a, row_a), (b, row_b)| {
        b.total_cmp(a)
            .then_with(|| row_a.stock_symbol.cmp(&row_b.stock_symbol))
    });

    let mut scored = Vec::with_capacity(universe);
    let mut group_start = 0;
    while group_start < weighted.len() {
        let value = weighted[group_start].0;
        let group_end = weighted[group_start..]
            .iter()
            .position(|(other, _)| other.total_cmp(&value) != Ordering::Equal)
            .map_or(weighted.len(), |len| group_start + len);
        // 名次＝比它強的家數＋1；分數＝比它弱的家數占其他家數的百分位。
        let rs_score = percentile_score(weighted.len() - group_end, universe);
        scored.extend(
            weighted[group_start..group_end]
                .iter()
                .map(|(_, row)| ScoredStock {
                    rank: group_start + 1,
                    rs_score,
                    weighted: value,
                    row: row.clone(),
                }),
        );
        group_start = group_end;
    }
    scored
}

/// 依市場、產業、代號篩選已計分的全體，依 `order` 排序後取前 `limit` 筆。
fn build_response(
    data_date: NaiveDate,
    params: &RelativeStrengthParams,
    market_id: i32,
    scored: Vec<ScoredStock>,
) -> RelativeStrengthResponse {
    let universe = scored.len();
    let mut items: Vec<RelativeStrengthItem> = scored
        .into_iter()
        .filter(|stock| {
            let row = &stock.row;
            (market_id == 0 || row.market_id == market_id)
                && params
                    .industry
                    .as_deref()
                    .is_none_or(|industry| row.industry == industry)
                && params
                    .symbol
                    .as_deref()
                    .is_none_or(|symbol| row.stock_symbol == symbol)
        })
        .map(|stock| RelativeStrengthItem {
            rank: stock.rank,
            market: market_label(stock.row.market_id).to_owned(),
            rs_score: stock.rs_score,
            weighted_return: round2(stock.weighted),
            return_20d: stock.row.return_20d.map(round2),
            return_60d: round2(stock.row.return_60d),
            return_120d: round2(stock.row.return_120d),
            return_240d: round2(stock.row.return_240d),
            closing_price: stock.row.closing_price,
            stock_symbol: stock.row.stock_symbol,
            name: stock.row.name,
            industry: stock.row.industry,
        })
        .collect();

    let order = params.order.as_deref().unwrap_or("desc");
    if order == "asc" {
        items.reverse();
    }
    let matched = items.len();
    items.truncate(usize::from(params.limit.unwrap_or(50)));
    RelativeStrengthResponse {
        data_as_of: data_date.to_string(),
        market: params.market.clone().unwrap_or_else(|| "all".to_owned()),
        order: order.to_owned(),
        universe,
        matched,
        items,
    }
}

/// 股票表市場 id 對應的 API 市場代碼。
pub(super) fn market_label(market_id: i32) -> &'static str {
    if market_id == 4 { "tpex" } else { "twse" }
}

/// 比它弱的家數 `weaker` 在全體 `total` 中的百分位，換算成 1–99 分（只有一檔時為 99）。
pub(super) fn percentile_score(weaker: usize, total: usize) -> u8 {
    if total <= 1 {
        return 99;
    }
    let rank = weaker as f64 / (total - 1) as f64;
    // rank 在 0–1 之間，換算後必在 1–99。
    (rank * 98.0).floor() as u8 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
    }

    fn row(symbol: &str, market_id: i32, industry: &str, returns: [f64; 3]) -> UniverseRow {
        UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol}名稱"),
            market_id,
            industry: industry.to_owned(),
            return_20d: None,
            return_60d: returns[0],
            return_120d: returns[1],
            return_240d: returns[2],
            closing_price: Some(10.0),
        }
    }

    fn params(
        market: Option<&str>,
        industry: Option<&str>,
        order: Option<&str>,
    ) -> RelativeStrengthParams {
        RelativeStrengthParams {
            date: None,
            market: market.map(str::to_owned),
            industry: industry.map(str::to_owned),
            symbol: None,
            order: order.map(str::to_owned),
            limit: None,
        }
    }

    /// 資料日與 20／60／120／240 日前的基準日；交易日不足為 None。
    #[test]
    fn base_dates_counts_back_trading_days() {
        let dates: Vec<NaiveDate> = (0..=240)
            .map(|i| day(2026, 10, 13) - chrono::Days::new(i))
            .collect();
        let bases = base_dates(&dates).expect("交易日足夠");
        assert_eq!(bases.end, day(2026, 10, 13));
        assert_eq!(bases.d20, day(2026, 9, 23));
        assert_eq!(bases.d60, day(2026, 8, 14));
        assert_eq!(bases.d120, day(2026, 6, 15));
        assert_eq!(bases.d240, day(2026, 2, 15));
        assert!(base_dates(&dates[1..]).is_none());
        assert!(base_dates(&[]).is_none());
    }

    /// 百分位分數：最弱 1、最強 99、只有一檔 99。
    #[test]
    fn percentile_score_maps_to_1_99() {
        assert_eq!(percentile_score(0, 5), 1);
        assert_eq!(percentile_score(4, 5), 99);
        assert_eq!(percentile_score(2, 5), 50);
        assert_eq!(percentile_score(0, 1), 99);
    }

    /// 以全體計分排名，篩選只影響輸出；同分同名次；asc 由弱到強；limit 截斷但 matched 不變。
    #[test]
    fn build_response_ranks_whole_universe_then_filters() {
        let rows = vec![
            row("A", 2, "半導體業", [10.0, 20.0, 30.0]),
            row("B", 4, "半導體業", [-10.0, -10.0, -10.0]),
            row("C", 2, "航運業", [0.0, 0.0, 0.0]),
            row("D", 4, "航運業", [0.0, 0.0, 0.0]),
            row("E", 2, "半導體業", [50.0, 50.0, 50.0]),
        ];
        let all = build_response(
            day(2026, 10, 13),
            &params(None, None, None),
            0,
            score_universe(rows.clone()),
        );
        assert_eq!(all.data_as_of, "2026-10-13");
        assert_eq!(all.market, "all");
        assert_eq!(all.order, "desc");
        assert_eq!(all.universe, 5);
        assert_eq!(all.matched, 5);
        let ranked: Vec<(&str, usize, u8)> = all
            .items
            .iter()
            .map(|item| (item.stock_symbol.as_str(), item.rank, item.rs_score))
            .collect();
        assert_eq!(
            ranked,
            [
                ("E", 1, 99),
                ("A", 2, 74),
                ("C", 3, 25),
                ("D", 3, 25),
                ("B", 5, 1)
            ]
        );
        // A：0.4×10 + 0.3×20 + 0.3×30 = 19。
        assert_eq!(all.items[1].weighted_return, 19.0);
        assert_eq!(all.items[1].market, "twse");
        assert_eq!(all.items[1].industry, "半導體業");

        let otc = build_response(
            day(2026, 10, 13),
            &params(Some("tpex"), None, Some("asc")),
            4,
            score_universe(rows.clone()),
        );
        assert_eq!(otc.order, "asc");
        let symbols: Vec<&str> = otc.items.iter().map(|i| i.stock_symbol.as_str()).collect();
        assert_eq!(symbols, ["B", "D"]);
        assert_eq!(otc.items[1].rank, 3, "名次仍以全體計算");

        let mut semi = params(None, Some("半導體業"), None);
        semi.limit = Some(1);
        let semi = build_response(day(2026, 10, 13), &semi, 0, score_universe(rows.clone()));
        assert_eq!(semi.matched, 3);
        assert_eq!(semi.items.len(), 1);
        assert_eq!(semi.items[0].stock_symbol, "E");

        let mut one = params(None, None, None);
        one.symbol = Some("C".to_owned());
        let one = build_response(day(2026, 10, 13), &one, 0, score_universe(rows));
        assert_eq!(one.items.len(), 1);
        assert_eq!((one.items[0].rank, one.items[0].rs_score), (3, 25));
    }
}
