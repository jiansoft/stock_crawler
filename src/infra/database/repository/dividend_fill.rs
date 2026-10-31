//! 填息分析的資料來源：`dividend` 的除權息事件與 `DailyQuotes` 的日收盤。
//!
//! - [`DividendFillQuery`]：單一股票，`/stocks/{symbol}/dividend-fill` 與持股填息通知共用。
//! - [`MarketFillQuery`]：全市場填息樣本，`/market/dividend-fill-stats` 與持股的同組基準用。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::dividend::fill::{DailyClose, DividendFillQuery, ExRightEvent};
use crate::domain::dividend::fill_stats::{
    FillSample, MATURE_TRADING_DAYS, MarketFillQuery, YieldBucket,
};
use crate::infra::database;

/// 每個除權息日一列。
///
/// 同一個除權息日可能有多列：季配股的年度彙總列、或同一期以季別與月別各記一次。
/// 每個日期只取一列，有各期列時不取彙總列（彙總列的金額是全年加總）。
const EX_RIGHT_EVENTS_SQL: &str = r#"
WITH cash AS (
    SELECT DISTINCT ON ("ex-dividend_date1") "ex-dividend_date1" AS ex_date, cash_dividend
    FROM dividend
    WHERE security_code = $1 AND "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' AND cash_dividend > 0
    ORDER BY "ex-dividend_date1", (quarter = ''), cash_dividend DESC
), stock AS (
    SELECT DISTINCT ON ("ex-dividend_date2") "ex-dividend_date2" AS ex_date, stock_dividend
    FROM dividend
    WHERE security_code = $1 AND "ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' AND stock_dividend > 0
    ORDER BY "ex-dividend_date2", (quarter = ''), stock_dividend DESC
)
SELECT ex_date, COALESCE(cash.cash_dividend, 0) AS cash_dividend,
    COALESCE(stock.stock_dividend, 0) AS stock_dividend
FROM cash FULL JOIN stock USING (ex_date)
ORDER BY ex_date DESC
"#;

/// 全市場（4 碼普通股）每次除權息的填息樣本，事件規則與 [`EX_RIGHT_EVENTS_SQL`] 相同。
///
/// 參數：`$1` 起日（不含）、`$2` 迄日（含）、`$3` 觀察期交易日數。日期字串先以
/// `pg_input_is_valid` 排除不存在的日期（例如 02-30），避免整批轉型失敗。填息目標價
/// `round(前一日收盤 / (1 + 股票股利 / 10), 2)` 與領域的 `fill_target` 相同，在資料庫端算完，
/// 只回傳每次事件一列。
const MARKET_FILL_SAMPLES_SQL: &str = r#"
WITH cash AS (
    SELECT DISTINCT ON (security_code, "ex-dividend_date1")
        security_code, "ex-dividend_date1"::date AS ex_date, cash_dividend
    FROM dividend
    WHERE security_code ~ '^[1-9]\d{3}$' AND cash_dividend > 0
        AND "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' AND pg_input_is_valid("ex-dividend_date1", 'date')
    ORDER BY security_code, "ex-dividend_date1", (quarter = ''), cash_dividend DESC
), stock AS (
    SELECT DISTINCT ON (security_code, "ex-dividend_date2")
        security_code, "ex-dividend_date2"::date AS ex_date, stock_dividend
    FROM dividend
    WHERE security_code ~ '^[1-9]\d{3}$' AND stock_dividend > 0
        AND "ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' AND pg_input_is_valid("ex-dividend_date2", 'date')
    ORDER BY security_code, "ex-dividend_date2", (quarter = ''), stock_dividend DESC
), events AS (
    SELECT security_code AS symbol, ex_date, cash.cash_dividend AS cash,
        COALESCE(stock.stock_dividend, 0) AS stock
    FROM cash LEFT JOIN stock USING (security_code, ex_date)
    WHERE ex_date > $1 AND ex_date <= $2
)
SELECT e.cash, pre.close AS pre_ex_close, after.observed, after.days_to_fill
FROM events e
CROSS JOIN LATERAL (
    SELECT "ClosingPrice" AS close FROM "DailyQuotes"
    WHERE stock_symbol = e.symbol AND "Date" < e.ex_date AND "Date" >= e.ex_date - 30
        AND "ClosingPrice" > 0
    ORDER BY "Date" DESC LIMIT 1
) pre
CROSS JOIN LATERAL (
    SELECT count(*)::int AS observed,
        (min(n) FILTER (WHERE close >= round(pre.close / (1 + e.stock / 10), 2)))::int AS days_to_fill
    FROM (
        SELECT row_number() OVER (ORDER BY "Date") AS n, "ClosingPrice" AS close
        FROM (
            SELECT "Date", "ClosingPrice" FROM "DailyQuotes"
            WHERE stock_symbol = e.symbol AND "Date" >= e.ex_date AND "ClosingPrice" > 0
            ORDER BY "Date" LIMIT $3
        ) window_rows
    ) numbered
) after
WHERE after.observed > 0
"#;

/// 填息分析資料來源之 PostgreSQL 實作。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgDividendFillRepository;

impl PgDividendFillRepository {
    /// 建立倉儲。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DividendFillQuery for PgDividendFillRepository {
    async fn fetch_latest_quote_date(&self, symbol: &str) -> Result<Option<NaiveDate>> {
        sqlx::query_scalar(r#"SELECT max("Date") FROM "DailyQuotes" WHERE stock_symbol = $1"#)
            .bind(symbol)
            .fetch_one(database::get_connection())
            .await
            .context("Failed to fetch latest quote date")
    }

    async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>> {
        let rows: Vec<(String, Decimal, Decimal)> = sqlx::query_as(EX_RIGHT_EVENTS_SQL)
            .bind(symbol)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch ex-right events")?;
        Ok(rows
            .into_iter()
            .filter_map(|(ex_date, cash_dividend, stock_dividend)| {
                Some(ExRightEvent {
                    ex_date: NaiveDate::parse_from_str(&ex_date, "%Y-%m-%d").ok()?,
                    cash_dividend,
                    stock_dividend,
                })
            })
            .collect())
    }

    async fn fetch_daily_closes(&self, symbol: &str, since: NaiveDate) -> Result<Vec<DailyClose>> {
        let rows: Vec<(NaiveDate, Decimal)> = sqlx::query_as(
            r#"SELECT "Date", "ClosingPrice" FROM "DailyQuotes" WHERE stock_symbol = $1 AND "Date" >= $2 ORDER BY "Date""#,
        )
        .bind(symbol)
        .bind(since)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch daily closes")?;
        Ok(rows
            .into_iter()
            .map(|(date, close)| DailyClose { date, close })
            .collect())
    }
}

#[async_trait]
impl MarketFillQuery for PgDividendFillRepository {
    async fn fetch_market_fill_samples(
        &self,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<FillSample>> {
        let rows: Vec<(Decimal, Decimal, i32, Option<i32>)> =
            sqlx::query_as(MARKET_FILL_SAMPLES_SQL)
                .bind(since)
                .bind(until)
                .bind(i64::from(MATURE_TRADING_DAYS))
                .fetch_all(database::get_connection())
                .await
                .context("Failed to fetch market dividend fill samples")?;
        Ok(rows
            .into_iter()
            .filter_map(|(cash, pre_ex_close, observed, days_to_fill)| {
                Some(FillSample {
                    bucket: YieldBucket::of(cash, pre_ex_close)?,
                    trading_days_to_fill: days_to_fill.and_then(|days| u32::try_from(days).ok()),
                    trading_days_observed: u32::try_from(observed).ok()?,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
