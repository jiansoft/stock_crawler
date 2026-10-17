//! 三大財務報表各自的數值欄位清單與每列綁定，欄位順序必須與綁定順序一致。

use sqlx::{Postgres, query_builder::Separated};

use super::sql::push_key;
use crate::domain::financial::statement::{
    BalanceSheet, CashFlowStatement, IncomeStatement, StatementPeriod,
};

/// `income_statement` 的數值欄位，順序必須與 [`push_income_statement`] 綁定順序一致。
pub(super) const INCOME_STATEMENT_COLUMNS: &[&str] = &[
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
pub(super) const BALANCE_SHEET_COLUMNS: &[&str] = &[
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
pub(super) const CASH_FLOW_STATEMENT_COLUMNS: &[&str] = &[
    "depreciation",
    "amortization",
    "operating_cash_flow",
    "investing_cash_flow",
    "financing_cash_flow",
    "free_cash_flow",
    "net_cash_flow",
];

pub(super) fn push_income_statement(
    mut row: Separated<'_, Postgres, &'static str>,
    s: &IncomeStatement,
) {
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

pub(super) fn push_balance_sheet(mut row: Separated<'_, Postgres, &'static str>, s: &BalanceSheet) {
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

pub(super) fn push_cash_flow_statement(
    mut row: Separated<'_, Postgres, &'static str>,
    s: &CashFlowStatement,
) {
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
