//! 三大財務報表倉儲的單元與整合測試。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::Row;

use super::rows::{BALANCE_SHEET_COLUMNS, CASH_FLOW_STATEMENT_COLUMNS, INCOME_STATEMENT_COLUMNS};
use super::sql::{insert_prefix, upsert_suffix};
use super::*;
use crate::core::declare::Quarter;
use crate::domain::financial::statement::StatementPeriod;

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
    let (values, _, _) = fetch_row("balance_sheet", BALANCE_SHEET_COLUMNS, "single", "Q4").await;
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
