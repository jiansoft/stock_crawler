//! BigGo 三大財務報表 DTO → 領域實體的轉譯（Yahoo 財報的備援）的測試。

use rust_decimal_macros::dec;

use super::*;
use crate::core::declare::Quarter;

fn statement<T>(kind: ReportKind, year: i32, quarter: Option<u8>, items: T) -> Statement<T> {
    Statement {
        stock_symbol: "2330".to_string(),
        kind,
        period: FiscalPeriod { year, quarter },
        items,
    }
}

/// 2330 2026Q2：元換算千元後與 Yahoo 來源的資料列逐欄相同，BigGo 沒有的欄位為 `None`。
#[test]
fn income_statement_converts_to_thousands() {
    let dto = statement(
        ReportKind::Single,
        2026,
        Some(2),
        IncomeItems {
            revenue: Some(1_270_380_250_000),
            gross_profit: Some(860_310_695_000),
            operating_expenses: Some(98_982_083_000),
            operating_income: Some(766_602_651_000),
            income_after_taxes: Some(706_780_923_000),
            eps: Some(dec!(27.25)),
        },
    );
    let income = BigGoFinancialReportAclMapper::income_statement(&dto).unwrap();
    assert_eq!(income.source, "biggo");
    assert_eq!(income.period, StatementPeriod::Single(Quarter::Q2));
    assert_eq!(income.revenue, Some(1_270_380_250));
    assert_eq!(income.operating_profit, Some(766_602_651));
    assert_eq!(income.net_income, Some(706_780_923));
    assert_eq!(income.eps, Some(dec!(27.25)));
    assert_eq!(income.owner_parent_profit, None);
    assert_eq!(income.selling_expenses, None);
}

#[test]
fn balance_sheet_maps_only_matching_fields() {
    let dto = statement(
        ReportKind::Single,
        2026,
        Some(2),
        BalanceItems {
            cash_and_cash_equivalents: Some(3_134_218_213_000),
            current_financial_assets_fvtpl: Some(226_375_000),
            total_assets: Some(9_375_654_727_000),
            total_liabilities: Some(2_901_183_746_000),
            capital_stock: Some(259_323_701_000),
            equity: Some(6_474_470_981_000),
            ..Default::default()
        },
    );
    let balance = BigGoFinancialReportAclMapper::balance_sheet(&dto).unwrap();
    assert_eq!(balance.source, "biggo");
    assert_eq!(balance.quarter, Quarter::Q2);
    assert_eq!(balance.cash_and_equivalents, Some(3_134_218_213));
    assert_eq!(balance.short_term_investments, Some(226_375));
    assert_eq!(balance.total_assets, Some(9_375_654_727));
    assert_eq!(balance.share_capital, Some(259_323_701));
    assert_eq!(balance.equity, Some(6_474_470_981));
    assert_eq!(balance.accounts_receivable, None);
    assert_eq!(balance.retained_earnings, None);
    assert_eq!(balance.book_value_per_share, None);
}

#[test]
fn cash_flow_statement_maps_annual_period() {
    let dto = statement(
        ReportKind::Annual,
        2025,
        None,
        CashFlowItems {
            operating_cash_flow: Some(2_274_975_625_000),
            net_cash_flow: Some(640_229_359_000),
            ..Default::default()
        },
    );
    let cash_flow = BigGoFinancialReportAclMapper::cash_flow_statement(&dto).unwrap();
    assert_eq!(cash_flow.period, StatementPeriod::Annual);
    assert_eq!(cash_flow.operating_cash_flow, Some(2_274_975_625));
    assert_eq!(cash_flow.net_cash_flow, Some(640_229_359));
    assert_eq!(cash_flow.depreciation, None);
}

/// 金額不是 1000 的倍數、年度帶季別、資產負債表不是單季，都整筆拒收。
#[test]
fn rejects_inconsistent_data() {
    let odd = statement(
        ReportKind::Single,
        2026,
        Some(2),
        CashFlowItems {
            operating_cash_flow: Some(1_500),
            ..Default::default()
        },
    );
    assert!(BigGoFinancialReportAclMapper::cash_flow_statement(&odd).is_err());

    let annual_with_quarter = statement(ReportKind::Annual, 2025, Some(4), IncomeItems::default());
    assert!(BigGoFinancialReportAclMapper::income_statement(&annual_with_quarter).is_err());

    let annual_balance = statement(ReportKind::Annual, 2025, None, BalanceItems::default());
    assert!(BigGoFinancialReportAclMapper::balance_sheet(&annual_balance).is_err());
}
