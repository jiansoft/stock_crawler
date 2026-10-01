//! 三大財務報表（損益表、資產負債表、現金流量表）的 PostgreSQL 倉儲實作。
//!
//! 三張表共用主鍵 `(stock_symbol, fiscal_year, period_type, quarter, source)` 與相同的
//! upsert 規則，差別只在數值欄位，因此以欄位清單驅動同一份 SQL 產生器。
//!
//! 刻意不使用 `COPY`：[`CopyIn`](crate::infra::database) 不支援 `ON CONFLICT`，
//! 重抓同一期別會整批因主鍵重複而失敗。

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{Postgres, QueryBuilder, query_builder::Separated};

use crate::domain::financial::{
    repository::FinancialReportRepository,
    statement::{BalanceSheet, CashFlowStatement, IncomeStatement, StatementPeriod},
};
use crate::infra::database;

/// 主鍵欄位，同時是 `ON CONFLICT` 的衝突目標。
const KEY_COLUMNS: &str = "stock_symbol, fiscal_year, period_type, quarter, source";

/// 每批列數。最寬的資產負債表每列 34 個參數，500 列約 1.7 萬個，遠低於 PostgreSQL 的 65535 上限。
const SAVE_BATCH_SIZE: usize = 500;

/// `income_statement` 的數值欄位，順序必須與 [`push_income_statement`] 綁定順序一致。
const INCOME_STATEMENT_COLUMNS: &[&str] = &[
    "revenue",
    "gross_profit",
    "selling_expenses",
    "admin_expenses",
    "rd_expenses",
    "operating_expenses",
    "operating_profit",
    "non_operating_income",
    "profit_before_tax",
    "net_income",
    "owner_parent_profit",
    "revenue_per_share",
    "operating_profit_per_share",
    "profit_before_tax_per_share",
    "eps",
    "bps",
];

/// `balance_sheet` 的數值欄位，順序必須與 [`push_balance_sheet`] 綁定順序一致。
const BALANCE_SHEET_COLUMNS: &[&str] = &[
    "cash_and_equivalents",
    "short_term_investments",
    "accounts_receivable",
    "inventory",
    "other_current_assets",
    "current_assets",
    "equity_and_other_investments",
    "property_plant_equipment",
    "right_of_use_asset",
    "non_current_assets",
    "total_assets",
    "long_term_investment",
    "short_term_investment",
    "other_assets",
    "short_term_debt",
    "short_term_bills_payable",
    "accounts_payable",
    "current_portion_of_long_term_liabilities",
    "current_liabilities",
    "long_term_liabilities",
    "bonds_payable",
    "other_liabilities",
    "non_current_liabilities",
    "total_liabilities",
    "share_capital",
    "retained_earnings",
    "equity",
    "book_value_per_share",
];

/// `cash_flow_statement` 的數值欄位，順序必須與 [`push_cash_flow_statement`] 綁定順序一致。
const CASH_FLOW_STATEMENT_COLUMNS: &[&str] = &[
    "depreciation",
    "amortization",
    "operating_cash_flow",
    "investing_cash_flow",
    "financing_cash_flow",
    "free_cash_flow",
    "net_cash_flow",
];

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
}

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

/// `INSERT INTO {table} AS t (主鍵, 數值欄位) `，後面接 `VALUES`。
///
/// 別名 `t` 讓 `ON CONFLICT DO UPDATE` 能引用既有列的舊值。
fn insert_prefix(table: &str, value_columns: &[&str]) -> String {
    format!(
        "INSERT INTO {table} AS t ({KEY_COLUMNS}, {}) ",
        value_columns.join(", ")
    )
}

/// `ON CONFLICT` 子句。
///
/// 數值欄位一律覆寫、`fetched_at` 一律更新；`updated_at` 只在任一數值欄位
/// `IS DISTINCT FROM` 舊值時才更新（`NULL` 與數字之間的變動也算），
/// 讓 `updated_at` 能反映來源的事後修正，而不是每次重抓都被刷新。
fn upsert_suffix(value_columns: &[&str]) -> String {
    let assignments = value_columns
        .iter()
        .map(|column| format!("{column} = EXCLUDED.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let current = value_columns
        .iter()
        .map(|column| format!("t.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let incoming = value_columns
        .iter()
        .map(|column| format!("EXCLUDED.{column}"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        " ON CONFLICT ({KEY_COLUMNS}) DO UPDATE SET {assignments}, fetched_at = now(), \
         updated_at = CASE WHEN ROW({current}) IS DISTINCT FROM ROW({incoming}) \
         THEN now() ELSE t.updated_at END"
    )
}

/// 綁定主鍵欄位，順序對應 [`KEY_COLUMNS`]。
fn push_key(
    row: &mut Separated<'_, Postgres, &'static str>,
    stock_symbol: &str,
    fiscal_year: i32,
    period: StatementPeriod,
    source: &str,
) {
    row.push_bind(stock_symbol)
        .push_bind(fiscal_year)
        .push_bind(period.period_type())
        .push_bind(period.quarter_code())
        .push_bind(source);
}

fn push_income_statement(mut row: Separated<'_, Postgres, &'static str>, s: &IncomeStatement) {
    push_key(
        &mut row,
        &s.stock_symbol,
        s.fiscal_year,
        s.period,
        &s.source,
    );
    row.push_bind(s.revenue)
        .push_bind(s.gross_profit)
        .push_bind(s.selling_expenses)
        .push_bind(s.admin_expenses)
        .push_bind(s.rd_expenses)
        .push_bind(s.operating_expenses)
        .push_bind(s.operating_profit)
        .push_bind(s.non_operating_income)
        .push_bind(s.profit_before_tax)
        .push_bind(s.net_income)
        .push_bind(s.owner_parent_profit)
        .push_bind(s.revenue_per_share)
        .push_bind(s.operating_profit_per_share)
        .push_bind(s.profit_before_tax_per_share)
        .push_bind(s.eps)
        .push_bind(s.bps);
}

fn push_balance_sheet(mut row: Separated<'_, Postgres, &'static str>, s: &BalanceSheet) {
    push_key(
        &mut row,
        &s.stock_symbol,
        s.fiscal_year,
        StatementPeriod::Single(s.quarter),
        &s.source,
    );
    row.push_bind(s.cash_and_equivalents)
        .push_bind(s.short_term_investments)
        .push_bind(s.accounts_receivable)
        .push_bind(s.inventory)
        .push_bind(s.other_current_assets)
        .push_bind(s.current_assets)
        .push_bind(s.equity_and_other_investments)
        .push_bind(s.property_plant_equipment)
        .push_bind(s.right_of_use_asset)
        .push_bind(s.non_current_assets)
        .push_bind(s.total_assets)
        .push_bind(s.long_term_investment)
        .push_bind(s.short_term_investment)
        .push_bind(s.other_assets)
        .push_bind(s.short_term_debt)
        .push_bind(s.short_term_bills_payable)
        .push_bind(s.accounts_payable)
        .push_bind(s.current_portion_of_long_term_liabilities)
        .push_bind(s.current_liabilities)
        .push_bind(s.long_term_liabilities)
        .push_bind(s.bonds_payable)
        .push_bind(s.other_liabilities)
        .push_bind(s.non_current_liabilities)
        .push_bind(s.total_liabilities)
        .push_bind(s.share_capital)
        .push_bind(s.retained_earnings)
        .push_bind(s.equity)
        .push_bind(s.book_value_per_share);
}

fn push_cash_flow_statement(mut row: Separated<'_, Postgres, &'static str>, s: &CashFlowStatement) {
    push_key(
        &mut row,
        &s.stock_symbol,
        s.fiscal_year,
        s.period,
        &s.source,
    );
    row.push_bind(s.depreciation)
        .push_bind(s.amortization)
        .push_bind(s.operating_cash_flow)
        .push_bind(s.investing_cash_flow)
        .push_bind(s.financing_cash_flow)
        .push_bind(s.free_cash_flow)
        .push_bind(s.net_cash_flow);
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use sqlx::Row;

    use super::*;
    use crate::core::declare::Quarter;

    const FAKE_SYMBOL: &str = "79979";
    /// 固定歷史年度，避免與真實資料重疊。
    const FAKE_YEAR: i32 = 1990;

    #[test]
    fn test_insert_prefix() {
        assert_eq!(
            insert_prefix("cash_flow_statement", &["depreciation", "amortization"]),
            "INSERT INTO cash_flow_statement AS t (stock_symbol, fiscal_year, period_type, quarter, source, depreciation, amortization) "
        );
    }

    #[test]
    fn test_upsert_suffix() {
        assert_eq!(
            upsert_suffix(&["depreciation", "amortization"]),
            " ON CONFLICT (stock_symbol, fiscal_year, period_type, quarter, source) DO UPDATE SET \
             depreciation = EXCLUDED.depreciation, amortization = EXCLUDED.amortization, \
             fetched_at = now(), updated_at = CASE WHEN ROW(t.depreciation, t.amortization) \
             IS DISTINCT FROM ROW(EXCLUDED.depreciation, EXCLUDED.amortization) \
             THEN now() ELSE t.updated_at END"
        );
    }

    /// 欄位清單與綁定數量不一致時 SQL 會在執行期才失敗，這裡先在編譯單元內擋下。
    #[test]
    fn test_value_column_counts_match_entities() {
        assert_eq!(INCOME_STATEMENT_COLUMNS.len(), 16);
        assert_eq!(BALANCE_SHEET_COLUMNS.len(), 28);
        assert_eq!(CASH_FLOW_STATEMENT_COLUMNS.len(), 7);
    }

    /// 每個數值欄位給不同的值，欄位順序錯位時讀回比對一定會失敗。
    fn income_statement(period: StatementPeriod, base: i64) -> IncomeStatement {
        IncomeStatement {
            stock_symbol: FAKE_SYMBOL.to_string(),
            fiscal_year: FAKE_YEAR,
            period,
            source: "yahoo".to_string(),
            revenue: Some(base + 1),
            gross_profit: Some(base + 2),
            selling_expenses: Some(base + 3),
            admin_expenses: Some(base + 4),
            rd_expenses: Some(base + 5),
            operating_expenses: Some(base + 6),
            operating_profit: Some(-(base + 7)),
            non_operating_income: Some(base + 8),
            profit_before_tax: Some(base + 9),
            net_income: Some(base + 10),
            owner_parent_profit: None,
            revenue_per_share: Some(dec!(12.01)),
            operating_profit_per_share: Some(dec!(-3.02)),
            profit_before_tax_per_share: Some(dec!(4.03)),
            eps: Some(dec!(2.84)),
            bps: Some(dec!(34.64)),
        }
    }

    fn balance_sheet(quarter: Quarter) -> BalanceSheet {
        let v = |n: i64| Some(13_700_000_000 + n);
        BalanceSheet {
            stock_symbol: FAKE_SYMBOL.to_string(),
            fiscal_year: FAKE_YEAR,
            quarter,
            source: "yahoo".to_string(),
            cash_and_equivalents: v(1),
            short_term_investments: v(2),
            accounts_receivable: v(3),
            inventory: v(4),
            other_current_assets: v(5),
            current_assets: v(6),
            equity_and_other_investments: v(7),
            property_plant_equipment: v(8),
            right_of_use_asset: v(9),
            non_current_assets: v(10),
            total_assets: v(11),
            long_term_investment: v(12),
            short_term_investment: v(13),
            other_assets: v(14),
            short_term_debt: v(15),
            short_term_bills_payable: v(16),
            accounts_payable: v(17),
            current_portion_of_long_term_liabilities: v(18),
            current_liabilities: v(19),
            long_term_liabilities: v(20),
            bonds_payable: v(21),
            other_liabilities: v(22),
            non_current_liabilities: v(23),
            total_liabilities: v(24),
            share_capital: v(25),
            retained_earnings: Some(-1),
            equity: None,
            book_value_per_share: Some(dec!(-0.5)),
        }
    }

    fn cash_flow_statement(period: StatementPeriod) -> CashFlowStatement {
        CashFlowStatement {
            stock_symbol: FAKE_SYMBOL.to_string(),
            fiscal_year: FAKE_YEAR,
            period,
            source: "yahoo".to_string(),
            depreciation: Some(51_416),
            amortization: Some(4_129),
            operating_cash_flow: Some(100),
            investing_cash_flow: Some(-200),
            financing_cash_flow: Some(300),
            free_cash_flow: Some(-100),
            net_cash_flow: None,
        }
    }

    async fn cleanup() {
        for table in ["income_statement", "balance_sheet", "cash_flow_statement"] {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM {table} WHERE stock_symbol = $1"
            )))
            .bind(FAKE_SYMBOL)
            .execute(database::get_connection())
            .await;
        }
    }

    /// 讀回指定期別的一列，回傳 (數值欄位轉文字, fetched_at, updated_at)。
    ///
    /// 數值以 `::text` 讀回，bigint 與 numeric 可統一比較；`NULL` 為 `None`。
    async fn fetch_row(
        table: &str,
        value_columns: &[&str],
        period_type: &str,
        quarter: &str,
    ) -> (Vec<Option<String>>, DateTime<Utc>, DateTime<Utc>) {
        let selects = value_columns
            .iter()
            .map(|column| format!("{column}::text AS {column}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {selects}, fetched_at, updated_at FROM {table} \
             WHERE stock_symbol = $1 AND fiscal_year = $2 AND period_type = $3 \
             AND quarter = $4 AND source = 'yahoo'"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(FAKE_SYMBOL)
            .bind(FAKE_YEAR)
            .bind(period_type)
            .bind(quarter)
            .fetch_one(database::get_connection())
            .await
            .expect("fetch saved row");

        let values = value_columns
            .iter()
            .map(|column| row.try_get::<Option<String>, _>(*column).expect("value"))
            .collect();

        (
            values,
            row.try_get("fetched_at").expect("fetched_at"),
            row.try_get("updated_at").expect("updated_at"),
        )
    }

    fn text(values: &[Option<i64>]) -> Vec<Option<String>> {
        values.iter().map(|v| v.map(|n| n.to_string())).collect()
    }

    fn decimal_text(values: &[Option<Decimal>]) -> Vec<Option<String>> {
        values.iter().map(|v| v.map(|n| n.to_string())).collect()
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_save_round_trips_every_column() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_save_round_trips_every_column：無資料庫連接");
            return;
        }

        let repo = PgFinancialReportRepository::new();
        cleanup().await;

        let single = income_statement(StatementPeriod::Single(Quarter::Q2), 1_000);
        let annual = income_statement(StatementPeriod::Annual, 9_000);
        assert_eq!(
            repo.save_income_statements(&[single.clone(), annual])
                .await
                .expect("save income statements"),
            2
        );
        let (values, _, _) =
            fetch_row("income_statement", INCOME_STATEMENT_COLUMNS, "single", "Q2").await;
        let mut expected = text(&[
            single.revenue,
            single.gross_profit,
            single.selling_expenses,
            single.admin_expenses,
            single.rd_expenses,
            single.operating_expenses,
            single.operating_profit,
            single.non_operating_income,
            single.profit_before_tax,
            single.net_income,
            single.owner_parent_profit,
        ]);
        expected.extend(decimal_text(&[
            single.revenue_per_share,
            single.operating_profit_per_share,
            single.profit_before_tax_per_share,
            single.eps,
            single.bps,
        ]));
        assert_eq!(values, expected);
        let (values, _, _) =
            fetch_row("income_statement", INCOME_STATEMENT_COLUMNS, "annual", "A").await;
        assert_eq!(values[0].as_deref(), Some("9001"));

        let sheet = balance_sheet(Quarter::Q4);
        assert_eq!(
            repo.save_balance_sheets(std::slice::from_ref(&sheet))
                .await
                .expect("save balance sheets"),
            1
        );
        let (values, _, _) =
            fetch_row("balance_sheet", BALANCE_SHEET_COLUMNS, "single", "Q4").await;
        let mut expected: Vec<Option<String>> = (1..=25)
            .map(|n| Some((13_700_000_000_i64 + n).to_string()))
            .collect();
        expected.extend([Some("-1".to_string()), None, Some("-0.50".to_string())]);
        assert_eq!(values, expected);

        let flow = cash_flow_statement(StatementPeriod::Annual);
        repo.save_cash_flow_statements(std::slice::from_ref(&flow))
            .await
            .expect("save cash flow statements");
        let (values, _, _) = fetch_row(
            "cash_flow_statement",
            CASH_FLOW_STATEMENT_COLUMNS,
            "annual",
            "A",
        )
        .await;
        assert_eq!(
            values,
            text(&[
                flow.depreciation,
                flow.amortization,
                flow.operating_cash_flow,
                flow.investing_cash_flow,
                flow.financing_cash_flow,
                flow.free_cash_flow,
                flow.net_cash_flow,
            ])
        );

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_save_touches_updated_at_only_when_values_change() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_save_touches_updated_at_only_when_values_change：無資料庫連接");
            return;
        }

        let repo = PgFinancialReportRepository::new();
        cleanup().await;

        let period = StatementPeriod::Single(Quarter::Q1);
        let mut flow = cash_flow_statement(period);
        let fetch = || {
            fetch_row(
                "cash_flow_statement",
                CASH_FLOW_STATEMENT_COLUMNS,
                "single",
                "Q1",
            )
        };

        repo.save_cash_flow_statements(std::slice::from_ref(&flow))
            .await
            .expect("first save");
        let (_, first_fetched, first_updated) = fetch().await;

        // 數值完全相同：只刷新 fetched_at。
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            repo.save_cash_flow_statements(std::slice::from_ref(&flow))
                .await
                .expect("same values"),
            1
        );
        let (_, second_fetched, second_updated) = fetch().await;
        assert!(second_fetched > first_fetched);
        assert_eq!(second_updated, first_updated);

        // NULL 變成數字也算變動。
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        flow.net_cash_flow = Some(7);
        repo.save_cash_flow_statements(std::slice::from_ref(&flow))
            .await
            .expect("changed values");
        let (values, third_fetched, third_updated) = fetch().await;
        assert_eq!(values[6].as_deref(), Some("7"));
        assert!(third_fetched > second_fetched);
        assert!(third_updated > second_updated);

        cleanup().await;
    }
}
