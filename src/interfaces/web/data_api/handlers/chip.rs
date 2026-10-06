//! `/stocks/{symbol}/chip` handler：單一股票的籌碼面。
//!
//! 資料來自 `chip_daily`、`holder_distribution`、`insider_holding`、`broker_flow`
//! 四張表（由 `app::backfill::chip` 排程寫入）。

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
    BrokerFlowDay, BrokerNet, ChipDay, ChipParams, ChipResponse, ChipStreak, ErrorBody, HolderWeek,
    InsiderSummary,
};

/// 千張大戶回傳的週數。
const HOLDER_WEEKS: i64 = 8;

/// `chip_daily` 的一列。
#[derive(sqlx::FromRow)]
struct ChipDailyRow {
    date: NaiveDate,
    foreign_net: Option<i64>,
    trust_net: Option<i64>,
    dealer_net: Option<i64>,
    margin_previous: Option<i64>,
    margin_balance: Option<i64>,
    short_previous: Option<i64>,
    short_balance: Option<i64>,
}

impl ChipDailyRow {
    fn into_dto(self) -> ChipDay {
        let total_net = match (self.foreign_net, self.trust_net, self.dealer_net) {
            (Some(foreign), Some(trust), Some(dealer)) => Some(foreign + trust + dealer),
            _ => None,
        };
        ChipDay {
            date: self.date.to_string(),
            foreign_net: self.foreign_net,
            trust_net: self.trust_net,
            dealer_net: self.dealer_net,
            total_net,
            margin_balance: self.margin_balance,
            margin_change: self
                .margin_balance
                .zip(self.margin_previous)
                .map(|(t, p)| t - p),
            short_balance: self.short_balance,
            short_change: self
                .short_balance
                .zip(self.short_previous)
                .map(|(t, p)| t - p),
        }
    }
}

/// `holder_distribution` 的一列。
#[derive(sqlx::FromRow)]
struct HolderRow {
    date: NaiveDate,
    major_holders: i64,
    major_percent: Decimal,
    total_holders: i64,
}

/// `insider_holding` 最新月份的合計。
#[derive(sqlx::FromRow)]
struct InsiderRow {
    month: NaiveDate,
    insiders: i64,
    shares: Decimal,
    pledged: Decimal,
    related_pledged: Decimal,
}

/// `broker_flow` 的一列；分點清單以 JSON 文字取出。
#[derive(sqlx::FromRow)]
struct BrokerFlowRow {
    date: NaiveDate,
    buy_total: i64,
    sell_total: i64,
    main_share: Decimal,
    buyers: String,
    sellers: String,
}

/// 查詢單一股票的籌碼面：每日法人與融資融券、連續買賣超、千張大戶、董監設質、主力進出。
///
/// # Errors
///
/// 參數不合法回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗
/// 時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/chip", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), ChipParams), responses((status = 200, body = ChipResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_chip(
    Path(symbol): Path<String>,
    Query(params): Query<ChipParams>,
) -> Response {
    let days = params.days.unwrap_or(20);
    if !(1..=120).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 120");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    let pool = database::get_connection();

    // 主鍵是 (date, stock_symbol)，依股票查歷史走 `chip_daily-stock_symbol-date-idx`。
    let daily: Vec<ChipDailyRow> = match sqlx::query_as(
        r#"SELECT "date", foreign_net, trust_net, dealer_net, margin_previous, margin_balance, short_previous, short_balance FROM chip_daily WHERE stock_symbol = $1 ORDER BY "date" DESC LIMIT $2"#,
    )
    .bind(&symbol)
    .bind(i64::from(days))
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    let holders: Vec<HolderRow> = match sqlx::query_as(
        r#"SELECT "date", major_holders, major_percent, total_holders FROM holder_distribution WHERE stock_symbol = $1 ORDER BY "date" DESC LIMIT $2"#,
    )
    .bind(&symbol)
    .bind(HOLDER_WEEKS)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    let insider: Option<InsiderRow> = match sqlx::query_as(
        r#"SELECT "month", count(*) AS insiders, sum(shares)::numeric AS shares, sum(pledged)::numeric AS pledged, sum(related_pledged)::numeric AS related_pledged FROM insider_holding WHERE stock_symbol = $1 AND "month" = (SELECT max("month") FROM insider_holding WHERE stock_symbol = $1) GROUP BY "month""#,
    )
    .bind(&symbol)
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => return database_error(error),
    };

    let broker: Option<BrokerFlowRow> = match sqlx::query_as(
        r#"SELECT "date", buy_total, sell_total, main_share, buyers::text AS buyers, sellers::text AS sellers FROM broker_flow WHERE stock_symbol = $1 ORDER BY "date" DESC LIMIT 1"#,
    )
    .bind(&symbol)
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(error) => return database_error(error),
    };

    let streak = ChipStreak {
        foreign_days: streak(daily.iter().map(|row| row.foreign_net)),
        trust_days: streak(daily.iter().map(|row| row.trust_net)),
    };
    let daily: Vec<ChipDay> = daily.into_iter().map(ChipDailyRow::into_dto).collect();
    Json(ChipResponse {
        stock_symbol: symbol,
        data_as_of: daily.first().map(|day| day.date.clone()),
        daily,
        streak,
        holder_distribution: holders
            .into_iter()
            .map(|row| HolderWeek {
                date: row.date.to_string(),
                major_holders: row.major_holders,
                major_percent: decimal_to_f64(Some(row.major_percent)),
                total_holders: row.total_holders,
            })
            .collect(),
        insider: insider.map(insider_summary),
        broker_flow: broker.map(broker_flow_day),
    })
    .into_response()
}

/// 從最新一天往回數連續同向（同正負號）的天數；正值為連買、負值為連賣。
///
/// 最新一天沒有資料或買賣超為 0 時回 0；遇到缺資料、0 或反向即停止。
fn streak(values: impl Iterator<Item = Option<i64>>) -> i64 {
    let mut values = values.peekable();
    let direction = match values.peek().copied().flatten() {
        Some(value) if value != 0 => value.signum(),
        _ => return 0,
    };
    let days = values
        .take_while(|value| value.is_some_and(|value| value.signum() == direction))
        .count();
    direction * i64::try_from(days).unwrap_or(i64::MAX)
}

fn insider_summary(row: InsiderRow) -> InsiderSummary {
    let to_i64 = |value: Decimal| value.to_string().parse::<i64>().unwrap_or_default();
    let pledge_percent = if row.shares.is_zero() {
        None
    } else {
        decimal_to_f64(Some(
            (row.pledged / row.shares * Decimal::ONE_HUNDRED).round_dp(2),
        ))
    };
    InsiderSummary {
        month: row.month.format("%Y-%m").to_string(),
        insiders: row.insiders,
        shares: to_i64(row.shares),
        pledged: to_i64(row.pledged),
        related_pledged: to_i64(row.related_pledged),
        pledge_percent,
    }
}

/// 解析 `broker_flow.buyers/sellers` 的 JSON；`share` 以字串存放（`rust_decimal` 的 serde 預設）。
fn parse_brokers(raw: &str) -> Vec<BrokerNet> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let number = |key: &str| item.get(key).and_then(serde_json::Value::as_i64);
            let share = match item.get("share") {
                Some(serde_json::Value::String(text)) => text.parse().ok(),
                Some(value) => value.as_f64(),
                None => None,
            };
            Some(BrokerNet {
                name: item.get("name")?.as_str()?.to_string(),
                buy: number("buy")?,
                sell: number("sell")?,
                net: number("net")?,
                share,
            })
        })
        .collect()
}

fn broker_flow_day(row: BrokerFlowRow) -> BrokerFlowDay {
    BrokerFlowDay {
        date: row.date.to_string(),
        main_net: row.buy_total - row.sell_total,
        main_share: decimal_to_f64(Some(row.main_share)),
        buyers: parse_brokers(&row.buyers),
        sellers: parse_brokers(&row.sellers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 連續天數只算到第一個缺資料、0 或反向為止。
    #[test]
    fn streak_counts_same_direction_from_the_latest_day() {
        assert_eq!(streak([Some(5), Some(1), Some(-2), Some(3)].into_iter()), 2);
        assert_eq!(streak([Some(-5), Some(-1), Some(-2)].into_iter()), -3);
        assert_eq!(streak([Some(5), None, Some(3)].into_iter()), 1);
        assert_eq!(streak([Some(0), Some(3)].into_iter()), 0);
        assert_eq!(streak([None, Some(3)].into_iter()), 0);
        assert_eq!(streak(std::iter::empty()), 0);
    }

    /// 融資融券增減由今日減前日；法人任一類缺資料時合計為 null。
    #[test]
    fn daily_row_computes_changes_and_totals() {
        let row = ChipDailyRow {
            date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
            foreign_net: Some(-1_672_231),
            trust_net: Some(154_586),
            dealer_net: Some(395_975),
            margin_previous: Some(30_135),
            margin_balance: Some(30_500),
            short_previous: None,
            short_balance: Some(49),
        };
        let day = row.into_dto();
        assert_eq!(day.total_net, Some(-1_121_670));
        assert_eq!(day.margin_change, Some(365));
        assert_eq!(day.short_change, None);
    }

    /// 分點的比重存成字串也要讀得出來；欄位不全的項目略過。
    #[test]
    fn parse_brokers_reads_string_shares() {
        let brokers = parse_brokers(
            r#"[{"name":"凱基-台北","buy":150,"sell":30,"net":120,"share":"6.98"},{"name":"缺欄位"}]"#,
        );
        assert_eq!(brokers.len(), 1);
        assert_eq!(brokers[0].name, "凱基-台北");
        assert_eq!(brokers[0].share, Some(6.98));
        assert!(parse_brokers("not json").is_empty());
    }

    /// 設質比例以設質 ÷ 持股計算，持股為 0 時為 null。
    #[test]
    fn insider_summary_computes_pledge_percent() {
        let row = |shares: i64, pledged: i64| InsiderRow {
            month: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            insiders: 18,
            shares: Decimal::from(shares),
            pledged: Decimal::from(pledged),
            related_pledged: Decimal::ZERO,
        };
        let summary = insider_summary(row(1_743_969_104, 860_000_000));
        assert_eq!(summary.month, "2026-08");
        assert_eq!(summary.pledge_percent, Some(49.31));
        assert_eq!(insider_summary(row(0, 0)).pledge_percent, None);
    }
}
