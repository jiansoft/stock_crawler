//! 填息分析的資料來源（[`DividendFillQuery`]）：`dividend` 的除權息事件與 `DailyQuotes` 的日收盤。
//!
//! `/stocks/{symbol}/dividend-fill` 與持股填息通知共用。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::dividend::fill::{DailyClose, DividendFillQuery, ExRightEvent};
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
