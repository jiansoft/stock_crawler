//! `/stocks/{symbol}/timeline` handler：個股事件依日期合併成一條時間軸。
//!
//! 事件來源與欄位見 [`sql::STOCK_TIMELINE_SQL`]；標題與說明由 [`describe`] 依種類組成，數字去掉多餘的 0
//! （例如 `4.50` → `4.5`），百分比帶正負號。

mod sql;

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use self::sql::STOCK_TIMELINE_SQL;
use super::{database_error, error_response, parse_optional_date};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, StockTimelineParams, StockTimelineResponse, TimelineEvent, TimelineKindCount,
};

/// 事件種類；同一天依此順序排列，也是 `counts` 的順序。
const KINDS: [&str; 10] = [
    "ex_dividend",
    "ex_right",
    "cash_payable",
    "stock_payable",
    "corporate_action",
    "monthly_revenue",
    "quarterly_report",
    "signal",
    "chip_streak",
    "insider",
];

/// [`STOCK_TIMELINE_SQL`] 的一列。
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
pub(super) struct TimelineRow {
    pub(super) date: NaiveDate,
    pub(super) kind: String,
    pub(super) period: Option<String>,
    pub(super) label: Option<String>,
    pub(super) note: Option<String>,
    pub(super) v1: Option<Decimal>,
    pub(super) v2: Option<Decimal>,
    pub(super) v3: Option<Decimal>,
    pub(super) total: i64,
    pub(super) count_kinds: Vec<String>,
    pub(super) count_values: Vec<i64>,
}

/// 查詢個股在期間內的事件時間軸：除權息與股利發放、減資／分割、月營收、季報、技術訊號、法人連續
/// 買賣超里程碑與董監持股變化，並列出迄日之後 `upcoming_days` 天內已公告的除權息、發放與減資。
///
/// # Errors
///
/// 日期格式錯誤、`days` 不在 1–730、`upcoming_days` 不在 0–180、`limit` 不在 1–500 或 `kinds` 含不認得的
/// 種類回 422；股票不存在回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。沒有事件回空清單。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/timeline", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), StockTimelineParams), responses((status = 200, body = StockTimelineResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_timeline(
    Path(symbol): Path<String>,
    Query(params): Query<StockTimelineParams>,
) -> Response {
    let unprocessable = |message: &str| error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    let to = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value.unwrap_or_else(|| Local::now().date_naive()),
        Err(message) => return unprocessable(message),
    };
    let days = params.days.unwrap_or(90);
    if !(1..=730).contains(&days) {
        return unprocessable("days 必須介於 1 至 730");
    }
    let upcoming_days = params.upcoming_days.unwrap_or(60);
    if upcoming_days > 180 {
        return unprocessable("upcoming_days 必須介於 0 至 180");
    }
    let limit = params.limit.unwrap_or(200);
    if !(1..=500).contains(&limit) {
        return unprocessable("limit 必須介於 1 至 500");
    }
    let kinds = match parse_kinds(params.kinds.as_deref()) {
        Ok(kinds) => kinds,
        Err(message) => return unprocessable(message),
    };
    let name: Option<(String,)> =
        match sqlx::query_as(r#"SELECT "Name" FROM stocks WHERE stock_symbol = $1"#)
            .bind(&symbol)
            .fetch_optional(database::get_connection())
            .await
        {
            Ok(name) => name,
            Err(error) => return database_error(error),
        };
    let Some((name,)) = name else {
        return error_response(StatusCode::NOT_FOUND, "找不到股票代號");
    };

    let from = to - chrono::Duration::days(i64::from(days) - 1);
    let upcoming_to = to + chrono::Duration::days(i64::from(upcoming_days));
    let rows: Vec<TimelineRow> = match sqlx::query_as(STOCK_TIMELINE_SQL)
        .bind(&symbol)
        .bind(from)
        .bind(to)
        .bind(upcoming_to)
        .bind(kinds.as_deref())
        .bind(i64::from(limit))
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    Json(build_timeline(symbol, name, from, to, upcoming_to, rows)).into_response()
}

/// 解析 `kinds`：逗號分隔、去掉空白與重複；空字串或省略為全部（`None`），不認得的種類回錯誤訊息。
pub(super) fn parse_kinds(value: Option<&str>) -> Result<Option<Vec<String>>, &'static str> {
    let mut kinds: Vec<String> = Vec::new();
    for kind in value.unwrap_or_default().split(',').map(str::trim) {
        if kind.is_empty() || kinds.iter().any(|existing| existing == kind) {
            continue;
        }
        if !KINDS.contains(&kind) {
            return Err(
                "kinds 必須為 ex_dividend、ex_right、cash_payable、stock_payable、corporate_action、\
                 monthly_revenue、quarterly_report、signal、chip_streak 或 insider（逗號分隔）",
            );
        }
        kinds.push(kind.to_owned());
    }
    Ok((!kinds.is_empty()).then_some(kinds))
}

/// 組成時間軸：事件由新到舊、同一天依 [`KINDS`] 順序；迄日之後的標記為 `upcoming`。
pub(super) fn build_timeline(
    symbol: String,
    name: String,
    from: NaiveDate,
    to: NaiveDate,
    upcoming_to: NaiveDate,
    mut rows: Vec<TimelineRow>,
) -> StockTimelineResponse {
    let total = rows.first().map_or(0, |row| row.total);
    let mut counts: Vec<TimelineKindCount> = rows
        .first()
        .map(|row| {
            row.count_kinds
                .iter()
                .zip(&row.count_values)
                .map(|(kind, count)| TimelineKindCount {
                    kind: kind.clone(),
                    count: *count,
                })
                .collect()
        })
        .unwrap_or_default();
    counts.sort_by_key(|count| kind_order(&count.kind));
    rows.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| kind_order(&a.kind).cmp(&kind_order(&b.kind)))
    });
    StockTimelineResponse {
        stock_symbol: symbol,
        name: Some(name),
        from: from.to_string(),
        to: to.to_string(),
        upcoming_to: upcoming_to.to_string(),
        total,
        counts,
        events: rows
            .into_iter()
            .map(|row| {
                let (title, detail) = describe(&row);
                TimelineEvent {
                    date: row.date.to_string(),
                    upcoming: row.date > to,
                    kind: row.kind,
                    title,
                    detail,
                    period: row.period,
                }
            })
            .collect(),
    }
}

fn kind_order(kind: &str) -> usize {
    KINDS
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(KINDS.len())
}

/// 依種類組成事件的標題與說明（欄位意義見 [`STOCK_TIMELINE_SQL`]）。
pub(super) fn describe(row: &TimelineRow) -> (String, Option<String>) {
    let period = row.period.as_deref().unwrap_or_default();
    let v1 = row.v1.map(number).unwrap_or_default();
    match row.kind.as_str() {
        "ex_dividend" => (
            format!("除息：現金股利 {v1} 元"),
            dividend_period(row.period.as_deref()),
        ),
        "ex_right" => (
            format!("除權：股票股利 {v1} 元"),
            dividend_period(row.period.as_deref()),
        ),
        "cash_payable" => (
            format!("現金股利發放 {v1} 元"),
            dividend_period(row.period.as_deref()),
        ),
        "stock_payable" => (
            format!("股票股利發放 {v1} 元"),
            dividend_period(row.period.as_deref()),
        ),
        "corporate_action" => {
            let action = match row.label.as_deref() {
                Some("capital_reduction") => "減資",
                Some("split") => "分割",
                Some("reverse_split") => "反分割",
                _ => "股數變動",
            };
            (
                format!("{action}生效（換發比例 {v1}）"),
                row.note.clone().filter(|note| !note.is_empty()),
            )
        }
        "monthly_revenue" => {
            let title = match row.v1 {
                Some(yoy) => format!("{period} 營收年增 {}%", signed(yoy)),
                None => format!("{period} 營收公布"),
            };
            let mut detail = Vec::new();
            if let Some(monthly) = row.v3 {
                detail.push(format!(
                    "營收 {} 億元",
                    number(monthly / Decimal::from(100_000))
                ));
            }
            if let Some(mom) = row.v2 {
                detail.push(format!("月增 {}%", signed(mom)));
            }
            (title, (!detail.is_empty()).then(|| detail.join("、")))
        }
        "quarterly_report" => (
            format!("{period} 財報：EPS {v1} 元"),
            row.v2.map(|roe| format!("ROE {}%", number(roe))),
        ),
        "signal" => (
            row.note
                .clone()
                .or_else(|| row.label.clone())
                .unwrap_or_else(|| "技術訊號".to_owned()),
            None,
        ),
        "chip_streak" => {
            let investor = match row.label.as_deref() {
                Some("trust") => "投信",
                _ => "外資",
            };
            let streak = row.v1.unwrap_or_default();
            let direction = if streak.is_sign_negative() {
                "連賣"
            } else {
                "連買"
            };
            (
                format!("{investor}{direction} {} 天", number(streak.abs())),
                row.v2.map(|net| {
                    format!(
                        "當天買賣超 {} 張",
                        signed((net / Decimal::from(1000)).round())
                    )
                }),
            )
        }
        "insider" => (
            format!("{period} 董監持股 {}%", signed(row.v1.unwrap_or_default())),
            row.v2.map(|ratio| {
                format!(
                    "質押比例 {}%（{} 個百分點）",
                    number(ratio),
                    signed(row.v3.unwrap_or_default())
                )
            }),
        ),
        other => (other.to_owned(), None),
    }
}

/// 股利所屬期間（`2026Q2` → 「2026 年 Q2 股利」、`2025` → 「2025 年度股利」）。
fn dividend_period(period: Option<&str>) -> Option<String> {
    let period = period?;
    Some(match period.split_at_checked(4) {
        Some((year, "")) => format!("{year} 年度股利"),
        Some((year, quarter)) => format!("{year} 年 {quarter} 股利"),
        None => format!("{period} 股利"),
    })
}

/// 四捨五入到小數兩位、去掉多餘的 0。
fn number(value: Decimal) -> String {
    let rounded = value.round_dp(2).normalize();
    if rounded.is_zero() {
        "0".to_owned()
    } else {
        rounded.to_string()
    }
}

/// [`number`] 加上正負號（0 不加）。
fn signed(value: Decimal) -> String {
    let text = number(value);
    if value.round_dp(2).is_sign_positive() && text != "0" {
        format!("+{text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests;
