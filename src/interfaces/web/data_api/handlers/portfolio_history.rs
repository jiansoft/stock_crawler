//! `/portfolio/history` handler：持股的時間加權報酬、月報酬、最大回撤與大盤比較。
//!
//! 每日市值來自 `daily_money_history_member`（收盤後計算，`member_id = 0` 為全體）；
//! 買進成本取 `stock_ownership_details`、賣出所得取 `sell_history`（排除已撤銷）；
//! 大盤取 `index` 的 TAIEX。計算規則在 [`crate::domain::portfolio::performance`]。
//!
//! 買賣的入帳日取「交易日」與「建立日」較晚者：事後補登的交易在補登前沒有反映在
//! 每日市值裡，記在交易日會讓那幾天的報酬失真。非交易日的買賣併入下一個交易日。
//! 查全部會員時，會員中途開始記錄的那天視為投入（見 `FLOWS_SQL`）。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

use super::{database_error, error_response};
use crate::domain::portfolio::performance::{self, IndexClose, ValuePoint};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioHistoryParams, PortfolioHistoryResponse,
};

/// 期間內每日市值，依日期由舊到新。
const VALUES_SQL: &str = r#"
SELECT date, market_value
FROM daily_money_history_member
WHERE member_id = $1
  AND date >= (SELECT max(date) FROM daily_money_history_member WHERE member_id = $1)
      - make_interval(months => $2)
ORDER BY date
"#;

/// `$2` 之後、`$3`（含）以前的買賣，依入帳日彙總；`$1 = 0` 為全部會員。
///
/// 每位會員的買賣只計「該會員市值歷史第一天之後」的部分——更早的部位已包含在他第一天的
/// 市值裡。查全部會員時，另把每位會員市值歷史的第一天當成一筆投入（金額＝當天市值）：
/// 會員中途開始記錄時，合計市值會憑空多出他的持股，不當成投入就會被算成報酬
/// （2026-04-02 會員 3 開始記錄，合計當天多出 24.3 萬）。
const FLOWS_SQL: &str = r#"
WITH starts AS (
    SELECT member_id, min(date) AS first_date
    FROM daily_money_history_member
    WHERE member_id > 0 AND ($1::bigint = 0 OR member_id = $1)
    GROUP BY member_id
), flows AS (
    SELECT member_id, false AS joined,
        GREATEST(date, (created_time AT TIME ZONE 'Asia/Taipei')::date) AS booked,
        -holding_cost AS buys, 0::numeric AS sells
    FROM stock_ownership_details
    WHERE $1::bigint = 0 OR member_id = $1
    UNION ALL
    SELECT member_id, false,
        GREATEST(date, (create_time AT TIME ZONE 'Asia/Taipei')::date),
        0::numeric, amount
    FROM sell_history
    WHERE NOT is_reverted AND ($1::bigint = 0 OR member_id = $1)
    UNION ALL
    SELECT h.member_id, true, h.date, h.market_value, 0::numeric
    FROM daily_money_history_member h
    JOIN starts s ON s.member_id = h.member_id AND s.first_date = h.date
    WHERE $1::bigint = 0
)
SELECT f.booked AS date, SUM(f.buys) AS buys, SUM(f.sells) AS sells
FROM flows f
JOIN starts s ON s.member_id = f.member_id
WHERE (f.joined OR f.booked > s.first_date)
  AND f.booked > $2 AND f.booked <= $3
GROUP BY f.booked
ORDER BY f.booked
"#;

/// 期間內（往前多抓幾天以找到基準日當天或之前的收盤）的加權指數。
const INDEX_SQL: &str = r#"
SELECT date, index AS close
FROM index
WHERE category = 'TAIEX' AND date >= $1 AND date <= $2 AND index > 0
ORDER BY date
"#;

/// 往基準日前多抓的指數天數，涵蓋長假。
const INDEX_LOOKBACK_DAYS: u64 = 15;

#[derive(sqlx::FromRow)]
struct ValueRow {
    date: NaiveDate,
    market_value: Decimal,
}

#[derive(sqlx::FromRow)]
struct FlowRow {
    date: NaiveDate,
    buys: Decimal,
    sells: Decimal,
}

#[derive(sqlx::FromRow)]
struct IndexRow {
    date: NaiveDate,
    close: Decimal,
}

/// 查詢持股在近幾個月的時間加權報酬、月報酬、最大回撤與同期大盤報酬。
///
/// 報酬只看股價、不含現金股利；買進與賣出的金額已從報酬中扣除。
///
/// # Errors
///
/// `member_id` 為負數或 `months` 不在 1–120 回 422、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。市值歷史不足兩個交易日時回 200 與 `null` 欄位。
#[utoipa::path(get, path = "/api/v1/portfolio/history", tag = "data-api", params(PortfolioHistoryParams), responses((status = 200, body = PortfolioHistoryResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn portfolio_history(Query(params): Query<PortfolioHistoryParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let months = params.months.unwrap_or(12);
    if !(1..=120).contains(&months) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "months 必須介於 1 至 120");
    }
    let pool = database::get_connection();

    let values: Vec<ValueRow> = match sqlx::query_as(VALUES_SQL)
        .bind(member_id)
        .bind(i32::from(months))
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let (Some(first), Some(last)) = (values.first(), values.last()) else {
        return Json(PortfolioHistoryResponse::new(member_id, None)).into_response();
    };
    let (from, to) = (first.date, last.date);

    let flows: Vec<FlowRow> = match sqlx::query_as(FLOWS_SQL)
        .bind(member_id)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let index: Vec<IndexRow> = match sqlx::query_as(INDEX_SQL)
        .bind(from - Days::new(INDEX_LOOKBACK_DAYS))
        .bind(to)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    let points = attach_flows(values, flows);
    let index: Vec<IndexClose> = index
        .into_iter()
        .map(|row| IndexClose {
            date: row.date,
            close: row.close,
        })
        .collect();
    Json(PortfolioHistoryResponse::new(
        member_id,
        performance::evaluate(&points, &index),
    ))
    .into_response()
}

/// 把每筆買賣併入入帳日當天或之後的第一個交易日；晚於最後一個交易日的不計。
fn attach_flows(values: Vec<ValueRow>, flows: Vec<FlowRow>) -> Vec<ValuePoint> {
    let mut points: Vec<ValuePoint> = values
        .into_iter()
        .map(|row| ValuePoint {
            date: row.date,
            market_value: row.market_value,
            buys: Decimal::ZERO,
            sells: Decimal::ZERO,
        })
        .collect();
    for flow in flows {
        let position = points.partition_point(|point| point.date < flow.date);
        if let Some(point) = points.get_mut(position) {
            point.buys += flow.buys;
            point.sells += flow.sells;
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, day).expect("有效日期")
    }

    /// 週末的買賣併入下一個交易日；同一交易日的多筆相加；超過最後一天的不計。
    #[test]
    fn attach_flows_moves_weekend_flows_to_next_trading_day() {
        let values = [day(24), day(27), day(28)]
            .into_iter()
            .map(|date| ValueRow {
                date,
                market_value: dec!(100),
            })
            .collect();
        let flows = vec![
            FlowRow {
                date: day(25),
                buys: dec!(10),
                sells: dec!(0),
            },
            FlowRow {
                date: day(27),
                buys: dec!(5),
                sells: dec!(3),
            },
            FlowRow {
                date: day(30),
                buys: dec!(99),
                sells: dec!(0),
            },
        ];
        let points = attach_flows(values, flows);
        assert_eq!(
            points
                .iter()
                .map(|point| (point.buys, point.sells))
                .collect::<Vec<_>>(),
            vec![(dec!(0), dec!(0)), (dec!(15), dec!(3)), (dec!(0), dec!(0))]
        );
    }
}
