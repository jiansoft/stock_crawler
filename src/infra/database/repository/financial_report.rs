//! 三大財務報表（損益表、資產負債表、現金流量表）的 PostgreSQL 倉儲實作。
//!
//! 三張表共用主鍵 `(stock_symbol, fiscal_year, period_type, quarter, source)` 與相同的
//! upsert 規則，差別只在數值欄位，因此以欄位清單驅動同一份 SQL 產生器。
//!
//! 刻意不使用 `COPY`：[`CopyIn`](crate::infra::database) 不支援 `ON CONFLICT`，
//! 重抓同一期別會整批因主鍵重複而失敗。
//!
//! 子模組：[`sql`]（主鍵、`INSERT` 前綴與 `ON CONFLICT` 子句）、[`rows`]（三張表的數值欄位與綁定）。

mod rows;
mod sql;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{Postgres, QueryBuilder, query_builder::Separated};

use self::rows::{
    BALANCE_SHEET_COLUMNS, CASH_FLOW_STATEMENT_COLUMNS, INCOME_STATEMENT_COLUMNS,
    push_balance_sheet, push_cash_flow_statement, push_income_statement,
};
use self::sql::{insert_prefix, upsert_suffix};
use crate::domain::financial::{
    repository::FinancialReportRepository,
    statement::{BalanceSheet, CashFlowStatement, IncomeStatement},
};
use crate::infra::database;

/// 每批列數。最寬的資產負債表每列 34 個參數，500 列約 1.7 萬個，遠低於 PostgreSQL 的 65535 上限。
const SAVE_BATCH_SIZE: usize = 500;

/// 對應資料表 `income_statement`、`balance_sheet`、`cash_flow_statement`。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgFinancialReportRepository;

impl PgFinancialReportRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FinancialReportRepository for PgFinancialReportRepository {
    async fn save_income_statements(&self, statements: &[IncomeStatement]) -> Result<u64> {
        upsert(
            "income_statement",
            INCOME_STATEMENT_COLUMNS,
            statements,
            push_income_statement,
        )
        .await
    }

    async fn save_balance_sheets(&self, sheets: &[BalanceSheet]) -> Result<u64> {
        upsert(
            "balance_sheet",
            BALANCE_SHEET_COLUMNS,
            sheets,
            push_balance_sheet,
        )
        .await
    }

    async fn save_cash_flow_statements(&self, statements: &[CashFlowStatement]) -> Result<u64> {
        upsert(
            "cash_flow_statement",
            CASH_FLOW_STATEMENT_COLUMNS,
            statements,
            push_cash_flow_statement,
        )
        .await
    }

    async fn fetch_lagging_symbols(&self, published_within_days: i32) -> Result<Vec<String>> {
        sqlx::query_scalar(LAGGING_SYMBOLS_SQL)
            .bind(published_within_days)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch symbols whose income statements lag the official reports")
    }
}

/// 官方季報近期新公布、損益表還沒有那一季的股票。
///
/// 以 `created_time`（只在第一次寫入時設定）當作官方公布日；同一檔取期間內最新一季，
/// Yahoo 與 BigGo 任一來源已有該季（或更新的季別）就不算落後。
const LAGGING_SYMBOLS_SQL: &str = r#"
WITH official AS (
    SELECT DISTINCT ON (security_code)
        security_code,
        "year"::int AS fiscal_year,
        quarter
    FROM financial_statement
    WHERE quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
      AND created_time >= now() - make_interval(days => $1)
    ORDER BY security_code, "year" DESC, quarter DESC
)
SELECT o.security_code
FROM official AS o
WHERE NOT EXISTS (
    SELECT 1
    FROM income_statement AS i
    WHERE i.source IN ('yahoo', 'biggo')
      AND i.stock_symbol = o.security_code
      AND i.period_type = 'single'
      AND (i.fiscal_year, i.quarter) >= (o.fiscal_year, o.quarter)
)
ORDER BY o.security_code
"#;

/// 分批執行多列 upsert，回傳受影響列數總和。
async fn upsert<T>(
    table: &str,
    value_columns: &[&str],
    rows: &[T],
    push_row: fn(Separated<'_, Postgres, &'static str>, &T),
) -> Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let prefix = insert_prefix(table, value_columns);
    let suffix = upsert_suffix(value_columns);
    let mut affected = 0;

    for chunk in rows.chunks(SAVE_BATCH_SIZE) {
        let mut builder = QueryBuilder::<Postgres>::new(&prefix);
        builder.push_values(chunk, push_row);
        builder.push(&suffix);

        affected += builder
            .build()
            .execute(database::get_connection())
            .await
            .with_context(|| format!("Failed to upsert {} rows into {table}", chunk.len()))?
            .rows_affected();
    }

    Ok(affected)
}

#[cfg(test)]
mod tests;
