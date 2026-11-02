use rust_decimal_macros::dec;

use super::*;

fn dto<T: Default>(
    report_period: ReportPeriod,
    quarter: Option<u8>,
    items: T,
) -> FinancialStatement<T> {
    FinancialStatement {
        stock_symbol: "8042".to_string(),
        report_period,
        fiscal_period: FiscalPeriod {
            year: 2026,
            quarter,
        },
        items,
    }
}

#[test]
fn test_to_thousands() {
    assert_eq!(to_thousands(None, "x").expect("none"), None);
    assert_eq!(
        to_thousands(Some(dec!(-52332000.00)), "x").expect("negative"),
        Some(-52_332)
    );
    assert_eq!(
        to_thousands(Some(dec!(13700000000000000)), "x").expect("13.7 兆"),
        Some(13_700_000_000_000)
    );
    assert_eq!(to_thousands(Some(dec!(0.00)), "x").expect("zero"), Some(0));

    let err = to_thousands(Some(dec!(150932500)), "revenue").expect_err("not multiple");
    assert!(err.to_string().contains("revenue"));
    assert!(to_thousands(Some(dec!(1000.5)), "x").is_err());
}

#[test]
fn test_statement_period() {
    let fiscal = |quarter| FiscalPeriod {
        year: 2026,
        quarter,
    };

    assert_eq!(
        statement_period(ReportPeriod::Quarter, fiscal(Some(2))).expect("single"),
        StatementPeriod::Single(Quarter::Q2)
    );
    assert_eq!(
        statement_period(ReportPeriod::CumulativeQuarter, fiscal(Some(4))).expect("cumulative"),
        StatementPeriod::Cumulative(Quarter::Q4)
    );
    assert_eq!(
        statement_period(ReportPeriod::Year, fiscal(None)).expect("annual"),
        StatementPeriod::Annual
    );
    assert!(statement_period(ReportPeriod::Year, fiscal(Some(4))).is_err());
    assert!(statement_period(ReportPeriod::Quarter, fiscal(None)).is_err());
    assert!(statement_period(ReportPeriod::Quarter, fiscal(Some(5))).is_err());
}

#[test]
fn test_income_statement_mapping() {
    let items = IncomeStatementItems {
        revenue: Some(dec!(2380422000.00)),
        operating_profit: Some(dec!(-184249000)),
        owner_parent_profit: None,
        eps: Some(dec!(2.84)),
        bps: Some(dec!(34.64)),
        debt_cost: Some(dec!(0.5)),
        ..Default::default()
    };

    let statement =
        YahooFinancialReportAclMapper::income_statement(&dto(ReportPeriod::Year, None, items))
            .expect("mapped");

    assert_eq!(statement.stock_symbol, "8042");
    assert_eq!(statement.fiscal_year, 2026);
    assert_eq!(statement.period, StatementPeriod::Annual);
    assert_eq!(statement.source, "yahoo");
    assert_eq!(statement.revenue, Some(2_380_422));
    assert_eq!(statement.operating_profit, Some(-184_249));
    assert_eq!(statement.owner_parent_profit, None);
    assert_eq!(statement.eps, Some(dec!(2.84)));
    assert_eq!(statement.bps, Some(dec!(34.64)));
}

#[test]
fn test_income_statement_rejects_fractional_thousand() {
    let items = IncomeStatementItems {
        net_income: Some(dec!(381320500)),
        ..Default::default()
    };

    let err = YahooFinancialReportAclMapper::income_statement(&dto(
        ReportPeriod::Quarter,
        Some(2),
        items,
    ))
    .expect_err("should reject");

    let message = format!("{err:#}");
    assert!(message.contains("8042"), "{message}");
    assert!(message.contains("net_income"), "{message}");
}

#[test]
fn test_balance_sheet_mapping() {
    let items = BalanceSheetItems {
        total_assets: Some(dec!(13700000000000000)),
        short_term_investments: Some(dec!(1000)),
        short_term_investment: Some(dec!(2000)),
        retained_earnings: Some(dec!(-3000)),
        net_worth: Some(dec!(-0.5)),
        ..Default::default()
    };

    let sheet =
        YahooFinancialReportAclMapper::balance_sheet(&dto(ReportPeriod::Quarter, Some(4), items))
            .expect("mapped");

    assert_eq!(sheet.quarter, Quarter::Q4);
    assert_eq!(sheet.total_assets, Some(13_700_000_000_000));
    assert_eq!(sheet.short_term_investments, Some(1));
    assert_eq!(sheet.short_term_investment, Some(2));
    assert_eq!(sheet.retained_earnings, Some(-3));
    assert_eq!(sheet.equity, None);
    assert_eq!(sheet.book_value_per_share, Some(dec!(-0.5)));
}

#[test]
fn test_balance_sheet_rejects_non_single_period() {
    let annual = dto(ReportPeriod::Year, None, BalanceSheetItems::default());
    assert!(YahooFinancialReportAclMapper::balance_sheet(&annual).is_err());

    let cumulative = dto(
        ReportPeriod::CumulativeQuarter,
        Some(2),
        BalanceSheetItems::default(),
    );
    assert!(YahooFinancialReportAclMapper::balance_sheet(&cumulative).is_err());
}

#[test]
fn test_cash_flow_statement_mapping() {
    let items = CashFlowItems {
        depreciation: Some(dec!(51416000.00)),
        amortization: Some(dec!(4129000.00)),
        investing_cash_flow: Some(dec!(-200000)),
        net_cash_flow: None,
        ..Default::default()
    };

    let flow = YahooFinancialReportAclMapper::cash_flow_statement(&dto(
        ReportPeriod::Quarter,
        Some(2),
        items,
    ))
    .expect("mapped");

    assert_eq!(flow.period, StatementPeriod::Single(Quarter::Q2));
    assert_eq!(flow.depreciation, Some(51_416));
    assert_eq!(flow.amortization, Some(4_129));
    assert_eq!(flow.investing_cash_flow, Some(-200));
    assert_eq!(flow.operating_cash_flow, None);
    assert_eq!(flow.net_cash_flow, None);
}
