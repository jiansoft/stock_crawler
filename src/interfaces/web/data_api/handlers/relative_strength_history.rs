//! `/stocks/{symbol}/relative-strength` handler：個股近 N 個交易日的相對強度走勢。
//!
//! 每個交易日的分數與 [`super::relative_strength`] 排行同一算法：近 N 日報酬取
//! `daily_forward_return` 基準日 T−N 的 N 日欄位，加權報酬＝0.4 × 近 60 日 ＋ 0.3 × 近 120 日
//! ＋ 0.3 × 近 240 日，分數是比它弱的家數在上市櫃全體中的百分位。每天都要以全體排名，
//! 一次查詢在資料庫內以視窗函式算完，只回傳這檔股票的列。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::relative_strength::percentile_score;
use super::{database_error, ensure_stock_exists, error_response, parse_optional_date, round2};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, RelativeStrengthCrossing, RelativeStrengthHistoryParams,
    RelativeStrengthHistoryResponse, RelativeStrengthPoint,
};

/// 穿越時要標記的分數門檻：80 為強勢、50 為中位。
const THRESHOLDS: [u8; 2] = [80, 50];

/// `$1`（含）之前最近 `$2` 個交易日，每天以上市櫃全體計分後，取股票 `$3` 的分數與名次。
///
/// 交易日曆取 `$2 + 240` 個交易日（回看 800 天足以涵蓋 120 + 240 個交易日），資料日 T 的
/// 近 N 日報酬＝往前第 N 個交易日（基準日）的 N 日報酬；`weaker` 為加權報酬比它低的家數，
/// 由 Rust 換算成 1–99 分。母體條件與排行相同：上市櫃、未暫停交易、三個天期的報酬都有。
/// 報酬還沒回填的交易日沒有任何列，自然略過。
const HISTORY_SQL: &str = r#"
WITH td AS (
    SELECT date, (ROW_NUMBER() OVER (ORDER BY date DESC) - 1)::int AS idx
    FROM (
        SELECT DISTINCT "Date" AS date
        FROM "DailyQuotes"
        WHERE "Date" <= $1 AND "Date" > $1 - 800
        ORDER BY 1 DESC
        LIMIT $2::int + 240
    ) recent
),
ends AS (
    SELECT e.date AS end_date, b60.date AS d60, b120.date AS d120, b240.date AS d240
    FROM td e
    JOIN td b60 ON b60.idx = e.idx + 60
    JOIN td b120 ON b120.idx = e.idx + 120
    JOIN td b240 ON b240.idx = e.idx + 240
    WHERE e.idx < $2::int
),
scored AS (
    SELECT e.end_date, r60.stock_symbol,
           0.4 * r60.return_60d::float8 + 0.3 * r120.return_120d::float8
               + 0.3 * r240.return_240d::float8 AS weighted
    FROM ends e
    JOIN daily_forward_return r60 ON r60.date = e.d60
    JOIN daily_forward_return r120 ON r120.date = e.d120 AND r120.stock_symbol = r60.stock_symbol
    JOIN daily_forward_return r240 ON r240.date = e.d240 AND r240.stock_symbol = r60.stock_symbol
    JOIN stocks s ON s.stock_symbol = r60.stock_symbol
    WHERE r60.return_60d IS NOT NULL
      AND r120.return_120d IS NOT NULL
      AND r240.return_240d IS NOT NULL
      AND s.stock_exchange_market_id IN (2, 4)
      AND NOT s."SuspendListing"
),
ranked AS (
    SELECT end_date, stock_symbol, weighted,
           RANK() OVER (PARTITION BY end_date ORDER BY weighted) - 1 AS weaker,
           RANK() OVER (PARTITION BY end_date ORDER BY weighted DESC) AS rank,
           COUNT(*) OVER (PARTITION BY end_date) AS universe
    FROM scored
)
SELECT end_date AS date, weighted, weaker, rank, universe
FROM ranked
WHERE stock_symbol = $3
ORDER BY end_date DESC
"#;

/// [`HISTORY_SQL`] 的一列。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct HistoryRow {
    date: NaiveDate,
    weighted: f64,
    weaker: i64,
    rank: i64,
    universe: i64,
}

/// 查詢個股近 N 個交易日的相對強度分數、名次與穿越 50／80 的轉折。
///
/// 分數與 `/market/relative-strength` 同一算法（以當天上市櫃全體計算）；上市未滿 240 個交易日、
/// 暫停交易或非上市櫃的股票沒有分數，回 200 與空清單。
///
/// # Errors
///
/// 日期格式錯誤或 `days` 不在 1–120 回 422；股票不存在回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/relative-strength", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), RelativeStrengthHistoryParams), responses((status = 200, body = RelativeStrengthHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn relative_strength_history(
    Path(symbol): Path<String>,
    Query(params): Query<RelativeStrengthHistoryParams>,
) -> Response {
    let until = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value.unwrap_or_else(|| Local::now().date_naive()),
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(60);
    if !(1..=120).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 120");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }

    let rows: Vec<HistoryRow> = match sqlx::query_as(HISTORY_SQL)
        .bind(until)
        .bind(i32::from(days))
        .bind(&symbol)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    Json(build_history(symbol, days, rows)).into_response()
}

/// 由新到舊的列組成回應：分數、期間高低點（同分取最近一次）、分數變化與門檻穿越。
fn build_history(
    symbol: String,
    days: u16,
    rows: Vec<HistoryRow>,
) -> RelativeStrengthHistoryResponse {
    let items: Vec<RelativeStrengthPoint> = rows
        .into_iter()
        .map(|row| RelativeStrengthPoint {
            date: row.date.to_string(),
            rs_score: percentile_score(
                usize::try_from(row.weaker).unwrap_or(0),
                usize::try_from(row.universe).unwrap_or(0),
            ),
            rank: row.rank,
            universe: row.universe,
            weighted_return: round2(row.weighted),
        })
        .collect();

    // 由新到舊走訪，只在嚴格更高／更低時更新，因此同分保留最近一次。
    let mut high: Option<&RelativeStrengthPoint> = None;
    let mut low: Option<&RelativeStrengthPoint> = None;
    for item in &items {
        if high.is_none_or(|best| item.rs_score > best.rs_score) {
            high = Some(item);
        }
        if low.is_none_or(|worst| item.rs_score < worst.rs_score) {
            low = Some(item);
        }
    }
    let score_change = items
        .first()
        .zip(items.last())
        .map(|(latest, earliest)| i16::from(latest.rs_score) - i16::from(earliest.rs_score));

    RelativeStrengthHistoryResponse {
        stock_symbol: symbol,
        data_as_of: items.first().map(|item| item.date.clone()),
        days,
        latest_score: items.first().map(|item| item.rs_score),
        high_score: high.map(|item| item.rs_score),
        high_date: high.map(|item| item.date.clone()),
        low_score: low.map(|item| item.rs_score),
        low_date: low.map(|item| item.date.clone()),
        score_change,
        crossings: crossings(&items),
        items,
    }
}

/// 相鄰兩個交易日（`items` 由新到舊）穿越 [`THRESHOLDS`] 的轉折，由新到舊；同一天同時穿越
/// 兩個門檻時兩筆都列，高門檻在前。
fn crossings(items: &[RelativeStrengthPoint]) -> Vec<RelativeStrengthCrossing> {
    let mut found = Vec::new();
    for pair in items.windows(2) {
        let (current, previous) = (&pair[0], &pair[1]);
        for threshold in THRESHOLDS {
            let direction = match (
                previous.rs_score >= threshold,
                current.rs_score >= threshold,
            ) {
                (false, true) => "up",
                (true, false) => "down",
                _ => continue,
            };
            found.push(RelativeStrengthCrossing {
                date: current.date.clone(),
                threshold,
                direction: direction.to_owned(),
                from_score: previous.rs_score,
                to_score: current.rs_score,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: u32, weaker: i64, universe: i64) -> HistoryRow {
        HistoryRow {
            date: NaiveDate::from_ymd_opt(2026, 10, day).expect("合法日期"),
            weighted: 12.345,
            weaker,
            rank: universe - weaker,
            universe,
        }
    }

    /// 分數由 weaker／universe 換算；高低點同分取最近一次；變化＝最新減最舊；穿越由新到舊。
    #[test]
    fn build_history_scores_extremes_and_crossings() {
        // 由新到舊：13 日 99、12 日 50、9 日 40、8 日 85、7 日 99（universe 101 → 分數＝weaker × 98 ÷ 100 + 1）。
        let rows = vec![
            row(13, 100, 101),
            row(12, 50, 101),
            row(9, 40, 101),
            row(8, 86, 101),
            row(7, 100, 101),
        ];
        let response = build_history("2330".to_owned(), 60, rows);
        let scores: Vec<u8> = response.items.iter().map(|item| item.rs_score).collect();
        assert_eq!(scores, [99, 50, 40, 85, 99]);
        assert_eq!(response.data_as_of.as_deref(), Some("2026-10-13"));
        assert_eq!(response.latest_score, Some(99));
        assert_eq!(
            (response.high_score, response.high_date.as_deref()),
            (Some(99), Some("2026-10-13")),
            "同分取最近一次"
        );
        assert_eq!(
            (response.low_score, response.low_date.as_deref()),
            (Some(40), Some("2026-10-09"))
        );
        assert_eq!(response.score_change, Some(0));
        assert_eq!(response.items[0].rank, 1);
        assert_eq!(response.items[0].universe, 101);
        assert_eq!(response.items[0].weighted_return, 12.35);

        let crossings: Vec<(&str, u8, &str, u8, u8)> = response
            .crossings
            .iter()
            .map(|c| {
                (
                    c.date.as_str(),
                    c.threshold,
                    c.direction.as_str(),
                    c.from_score,
                    c.to_score,
                )
            })
            .collect();
        assert_eq!(
            crossings,
            [
                ("2026-10-13", 80, "up", 50, 99),
                ("2026-10-12", 50, "up", 40, 50),
                ("2026-10-09", 80, "down", 85, 40),
                ("2026-10-09", 50, "down", 85, 40),
            ]
        );
    }

    /// 期間內沒有分數：所有摘要欄位為 null、清單為空。
    #[test]
    fn build_history_empty() {
        let response = build_history("7799".to_owned(), 20, Vec::new());
        assert_eq!(response.days, 20);
        assert!(response.data_as_of.is_none());
        assert!(response.latest_score.is_none());
        assert!(response.high_date.is_none());
        assert!(response.low_score.is_none());
        assert!(response.score_change.is_none());
        assert!(response.crossings.is_empty());
        assert!(response.items.is_empty());
    }
}
