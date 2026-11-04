//! 持股股利預估的輸入查詢與每月快照（`/portfolio/dividend-forecast` 與 08:15 快照排程共用），
//! 以及快照與實際入帳的誤差比對輸入（`/portfolio/dividend-forecast/accuracy`）。
//!
//! 持股取 `stock_ownership_details`（未賣出，依代號合併），配息取 `dividend` 近兩年的現金股利列，
//! 報價取 `last_daily_quotes`；已除息未發放的取已領股利明細（[`super::payout`]）。

use anyhow::{Context, Result};
use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::Decimal;
use sqlx::QueryBuilder;

use crate::domain::portfolio::dividend_forecast::{
    self, DividendEvent, Entitlement, ForecastItem, Holding,
};
use crate::domain::portfolio::forecast_accuracy::{self, ActualPayout, ForecastedPayout};
use crate::infra::database;

/// 目前持股（依代號合併）與最新收盤價；`$1 = 0` 時為全部會員。
pub(crate) const HOLDINGS_SQL: &str = r#"
SELECT o.security_code AS stock_symbol, COALESCE(s."Name", '') AS name,
    SUM(o.share_quantity)::bigint AS shares,
    MAX(CASE WHEN q.closing_price > 0 THEN q.closing_price END) AS closing_price
FROM stock_ownership_details o
LEFT JOIN stocks s ON s.stock_symbol = o.security_code
LEFT JOIN last_daily_quotes q ON q.stock_symbol = o.security_code
WHERE NOT o.is_sold AND ($1::bigint = 0 OR o.member_id = $1)
GROUP BY o.security_code, s."Name"
HAVING SUM(o.share_quantity) > 0
ORDER BY o.security_code
"#;

/// 指定代號自 `$2` 年起的現金股利列；日期欄位不是 `YYYY-MM-DD`（`-`、`尚未公布`）時為 NULL。
const EVENTS_SQL: &str = r#"
SELECT security_code AS stock_symbol, year_of_dividend, quarter, cash_dividend,
    CASE WHEN "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date1"::date END AS ex_date,
    CASE WHEN payable_date1 ~ '^\d{4}-\d{2}-\d{2}$' THEN payable_date1::date END AS payable_date
FROM dividend
WHERE security_code = ANY($1) AND cash_dividend > 0 AND year >= $2
"#;

/// [`HOLDINGS_SQL`] 的一列；`/portfolio/upcoming-dividends` 也用同一個查詢取持股股數。
#[derive(sqlx::FromRow)]
pub(crate) struct HoldingRow {
    pub(crate) stock_symbol: String,
    name: String,
    pub(crate) shares: i64,
    closing_price: Option<Decimal>,
}

impl From<HoldingRow> for Holding {
    fn from(row: HoldingRow) -> Self {
        Self {
            stock_symbol: row.stock_symbol,
            name: row.name,
            shares: row.shares,
            closing_price: row.closing_price,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    stock_symbol: String,
    year_of_dividend: i32,
    quarter: String,
    cash_dividend: Decimal,
    ex_date: Option<NaiveDate>,
    payable_date: Option<NaiveDate>,
}

impl From<EventRow> for DividendEvent {
    fn from(row: EventRow) -> Self {
        Self {
            stock_symbol: row.stock_symbol,
            year_of_dividend: row.year_of_dividend,
            quarter: row.quarter,
            cash_per_share: row.cash_dividend,
            ex_date: row.ex_date,
            payable_date: row.payable_date,
        }
    }
}

/// 預估一位會員（0 為全部會員）所需的持股、配息列與已除息的已領明細。
#[derive(Debug, Default)]
pub(crate) struct ForecastInputs {
    pub(crate) holdings: Vec<Holding>,
    pub(crate) events: Vec<DividendEvent>,
    pub(crate) entitlements: Vec<Entitlement>,
}

/// 讀取 `today` 起 `months` 個月預估的輸入。
///
/// # Errors
///
/// 任一查詢失敗時回傳錯誤。
pub(crate) async fn load_inputs(
    member_id: i64,
    today: NaiveDate,
    months: u32,
) -> Result<ForecastInputs> {
    let pool = database::get_connection();
    let holdings: Vec<HoldingRow> = sqlx::query_as(HOLDINGS_SQL)
        .bind(member_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("查詢預估用持股失敗: member_id={member_id}"))?;
    let symbols: Vec<&str> = holdings
        .iter()
        .map(|row| row.stock_symbol.as_str())
        .collect();
    let events: Vec<EventRow> = sqlx::query_as(EVENTS_SQL)
        .bind(&symbols)
        .bind(today.year() - 2)
        .fetch_all(pool)
        .await
        .with_context(|| format!("查詢預估用股利失敗: member_id={member_id}"))?;
    let end = today
        .checked_add_months(Months::new(months))
        .unwrap_or(today);
    let allocations = super::payout::fetch(member_id, today + Days::new(1), end, today).await?;
    Ok(ForecastInputs {
        holdings: holdings.into_iter().map(Holding::from).collect(),
        events: events.into_iter().map(DividendEvent::from).collect(),
        entitlements: dividend_forecast::entitlements(&allocations),
    })
}

/// 有未賣出持股的會員，依編號排序。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub(crate) async fn fetch_member_ids() -> Result<Vec<i64>> {
    sqlx::query_scalar(
        "SELECT DISTINCT member_id FROM stock_ownership_details WHERE NOT is_sold AND share_quantity > 0 ORDER BY member_id",
    )
    .fetch_all(database::get_connection())
    .await
    .context("查詢有持股的會員失敗")
}

/// 以 `items`（會員, 預估）整批替換 `snapshot_date` 的快照，回傳寫入筆數。
///
/// 刪除與寫入在同一個交易內；同一會員、代號、除息日重複時保留第一筆。
///
/// # Errors
///
/// 刪除或寫入失敗時回傳錯誤，交易回滾、原有快照不變。
pub(crate) async fn replace_snapshot(
    snapshot_date: NaiveDate,
    items: &[(i64, ForecastItem)],
) -> Result<u64> {
    let mut tx = database::get_tx().await?;
    sqlx::query("DELETE FROM dividend_forecast_snapshot WHERE snapshot_date = $1")
        .bind(snapshot_date)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("刪除股利預估快照失敗: {snapshot_date}"))?;
    let mut inserted = 0;
    for chunk in items.chunks(1_000) {
        let mut builder = QueryBuilder::new(
            "INSERT INTO dividend_forecast_snapshot (snapshot_date, member_id, stock_symbol, ex_date, pay_date, shares, cash_per_share, amount, basis, entitled) ",
        );
        builder.push_values(chunk, |mut row, (member_id, item)| {
            row.push_bind(snapshot_date)
                .push_bind(member_id)
                .push_bind(&item.stock_symbol)
                .push_bind(item.ex_date)
                .push_bind(item.pay_date)
                .push_bind(item.shares)
                .push_bind(item.cash_per_share)
                .push_bind(item.amount)
                .push_bind(item.basis.as_str())
                .push_bind(item.entitled);
        });
        builder.push(" ON CONFLICT DO NOTHING");
        inserted += builder
            .build()
            .execute(&mut *tx)
            .await
            .with_context(|| format!("寫入股利預估快照失敗: {snapshot_date}"))?
            .rows_affected();
    }
    tx.commit()
        .await
        .with_context(|| format!("提交股利預估快照失敗: {snapshot_date}"))?;
    Ok(inserted)
}

/// 快照中的逐筆預估；`$2 = 0` 時為全部會員。
const SNAPSHOT_SQL: &str = r#"
SELECT f.stock_symbol, COALESCE(s."Name", '') AS name, f.pay_date, f.amount, f.basis
FROM dividend_forecast_snapshot AS f
LEFT JOIN stocks AS s ON s.stock_symbol = f.stock_symbol
WHERE f.snapshot_date = $1 AND ($2::bigint = 0 OR f.member_id = $2)
"#;

/// [`SNAPSHOT_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct SnapshotRow {
    stock_symbol: String,
    name: String,
    pay_date: NaiveDate,
    amount: Decimal,
    basis: String,
}

impl From<SnapshotRow> for ForecastedPayout {
    fn from(row: SnapshotRow) -> Self {
        Self {
            stock_symbol: row.stock_symbol,
            name: row.name,
            pay_date: row.pay_date,
            amount: row.amount,
            basis: row.basis,
        }
    }
}

/// 預估誤差比對的輸入：快照的逐筆預估與快照後已入帳的現金股利。
#[derive(Debug, Default)]
pub(crate) struct AccuracyInputs {
    pub(crate) forecasts: Vec<ForecastedPayout>,
    pub(crate) actuals: Vec<ActualPayout>,
}

/// 最近一次快照日；還沒有任何快照時為 `None`。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub(crate) async fn latest_snapshot_date() -> Result<Option<NaiveDate>> {
    sqlx::query_scalar("SELECT max(snapshot_date) FROM dividend_forecast_snapshot")
        .fetch_one(database::get_connection())
        .await
        .context("查詢最近一次股利預估快照日失敗")
}

/// 不晚於 `date` 的最近一次快照日；沒有時為 `None`。
///
/// 每月 1 日 08:10 的上月入帳通知用它找「上個月底前」的快照：當天 08:15 的新快照
/// 涵蓋的是快照隔天起的預估，不含上個月，不能拿來比對（N59）。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub(crate) async fn latest_snapshot_on_or_before(date: NaiveDate) -> Result<Option<NaiveDate>> {
    sqlx::query_scalar(
        "SELECT max(snapshot_date) FROM dividend_forecast_snapshot WHERE snapshot_date <= $1",
    )
    .bind(date)
    .fetch_one(database::get_connection())
    .await
    .with_context(|| format!("查詢 {date} 以前最近一次股利預估快照日失敗"))
}

/// 讀取 `snapshot_date` 快照的預估，與快照隔天起到 `today`（最多快照涵蓋的 12 個月）已入帳的現金股利。
///
/// # Errors
///
/// 任一查詢失敗時回傳錯誤。
pub(crate) async fn load_accuracy_inputs(
    snapshot_date: NaiveDate,
    member_id: i64,
    today: NaiveDate,
) -> Result<AccuracyInputs> {
    let rows: Vec<SnapshotRow> = sqlx::query_as(SNAPSHOT_SQL)
        .bind(snapshot_date)
        .bind(member_id)
        .fetch_all(database::get_connection())
        .await
        .with_context(|| format!("查詢股利預估快照失敗: {snapshot_date}"))?;
    let from = snapshot_date + Days::new(1);
    let to = snapshot_date
        .checked_add_months(Months::new(12))
        .unwrap_or(snapshot_date)
        .min(today);
    let allocations = if from <= to {
        super::payout::fetch(member_id, from, to, today).await?
    } else {
        Vec::new()
    };
    Ok(AccuracyInputs {
        forecasts: rows.into_iter().map(ForecastedPayout::from).collect(),
        actuals: forecast_accuracy::actual_payouts(&allocations),
    })
}
