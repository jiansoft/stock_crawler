//! `/stocks/{symbol}/holder-levels` handler：單一股票單週的集保股權分散全部持股分級。
//!
//! 資料來自 `holder_distribution_level`（`app::backfill::chip` 每週六入庫）。集保開放資料只有最新一週，
//! 無法回補，2026-10-22 起逐週累積；更早的週只有 `/stocks/{symbol}/chip` 的千張與 400 張摘要。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{database_error, decimal_to_f64, ensure_stock_exists, error_response};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, HolderLevelItem, HolderLevelsParams, HolderLevelsResponse,
};

/// 差異數調整的分級代碼。
const LEVEL_ADJUSTMENT: i16 = 16;
/// 合計的分級代碼。
const LEVEL_TOTAL: i16 = 17;

/// 集保分級 1～17 的名稱（依集保戶股權分散表）。
const LEVEL_LABELS: [&str; 17] = [
    "1～999 股",
    "1,000～5,000 股",
    "5,001～10,000 股",
    "10,001～15,000 股",
    "15,001～20,000 股",
    "20,001～30,000 股",
    "30,001～40,000 股",
    "40,001～50,000 股",
    "50,001～100,000 股",
    "100,001～200,000 股",
    "200,001～400,000 股",
    "400,001～600,000 股",
    "600,001～800,000 股",
    "800,001～1,000,000 股",
    "1,000,001 股以上",
    "差異數調整",
    "合計",
];

/// 指定股票在 `$2`（含）以前最近一週的全部分級；`$2` 為 null 時取最新一週。
const LEVELS_SQL: &str = r#"
SELECT "date", level, holders, shares, percent
FROM holder_distribution_level
WHERE stock_symbol = $1
  AND "date" = (
    SELECT max("date") FROM holder_distribution_level
    WHERE stock_symbol = $1 AND ($2::date IS NULL OR "date" <= $2)
  )
ORDER BY level
"#;

/// `holder_distribution_level` 的一列。
#[derive(sqlx::FromRow)]
struct LevelRow {
    date: NaiveDate,
    level: i16,
    holders: i64,
    shares: i64,
    percent: Decimal,
}

/// 分級名稱；超出 1～17 時為「未知級距」。
fn level_label(level: i16) -> &'static str {
    usize::try_from(level)
        .ok()
        .and_then(|level| level.checked_sub(1))
        .and_then(|index| LEVEL_LABELS.get(index))
        .copied()
        .unwrap_or("未知級距")
}

/// 把同一週的分級列組成回應；沒有任何列時回 `None`。
fn build_response(stock_symbol: String, rows: Vec<LevelRow>) -> Option<HolderLevelsResponse> {
    let date = rows.first()?.date;
    let mut response = HolderLevelsResponse {
        stock_symbol,
        date: date.to_string(),
        levels: Vec::with_capacity(15),
        adjustment: None,
        total: None,
    };
    for row in rows {
        let item = HolderLevelItem {
            level: row.level,
            label: level_label(row.level).to_owned(),
            holders: row.holders,
            shares: row.shares,
            percent: decimal_to_f64(Some(row.percent)),
        };
        match row.level {
            LEVEL_ADJUSTMENT => response.adjustment = Some(item),
            LEVEL_TOTAL => response.total = Some(item),
            _ => response.levels.push(item),
        }
    }
    Some(response)
}

/// 查詢單一股票單週的集保股權分散全部持股分級（1～15 級、差異數調整、合計）。
///
/// # Errors
///
/// `date` 格式不合法回 422、股票不存在或該期間沒有分級資料回 404、驗證失敗回 401；
/// 資料庫查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/holder-levels", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), HolderLevelsParams), responses((status = 200, body = HolderLevelsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn holder_levels(
    Path(symbol): Path<String>,
    Query(params): Query<HolderLevelsParams>,
) -> Response {
    let as_of = match params.date.as_deref() {
        None => None,
        Some(raw) => match NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            Ok(date) => Some(date),
            Err(_) => {
                return error_response(StatusCode::UNPROCESSABLE_ENTITY, "date 必須為 YYYY-MM-DD");
            }
        },
    };
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    let rows: Vec<LevelRow> = match sqlx::query_as(LEVELS_SQL)
        .bind(&symbol)
        .bind(as_of)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    match build_response(symbol, rows) {
        Some(response) => Json(response).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            "沒有集保持股分級資料（2026-10-22 起逐週累積）",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(level: i16, holders: i64, percent: Decimal) -> LevelRow {
        LevelRow {
            date: NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(),
            level,
            holders,
            shares: holders * 1_000,
            percent,
        }
    }

    #[test]
    fn level_labels_cover_every_tdcc_level() {
        assert_eq!(level_label(1), "1～999 股");
        assert_eq!(level_label(12), "400,001～600,000 股");
        assert_eq!(level_label(15), "1,000,001 股以上");
        assert_eq!(level_label(16), "差異數調整");
        assert_eq!(level_label(17), "合計");
        assert_eq!(level_label(0), "未知級距");
        assert_eq!(level_label(18), "未知級距");
        assert_eq!(level_label(-1), "未知級距");
    }

    /// 1～15 級依序放進 levels，16 與 17 分開；沒有列時回 `None`。
    #[test]
    fn build_response_splits_adjustment_and_total() {
        let response = build_response(
            "2330".to_owned(),
            vec![
                row(1, 1_000, Decimal::new(125, 2)),
                row(15, 1_482, Decimal::new(847_800, 4)),
                row(16, 2, Decimal::ZERO),
                row(17, 3_010_913, Decimal::ONE_HUNDRED),
            ],
        )
        .expect("有分級資料");
        assert_eq!(response.date, "2026-10-22");
        assert_eq!(response.levels.len(), 2);
        assert_eq!(response.levels[1].label, "1,000,001 股以上");
        assert_eq!(response.levels[1].percent, Some(84.78));
        assert_eq!(
            response.adjustment.as_ref().map(|item| item.holders),
            Some(2)
        );
        assert_eq!(
            response.total.as_ref().map(|item| item.label.as_str()),
            Some("合計")
        );
        assert_eq!(build_response("2330".to_owned(), Vec::new()), None);
    }
}
