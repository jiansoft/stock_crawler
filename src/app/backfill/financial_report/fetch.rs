//! 單檔三大報表的抓取與轉譯：Yahoo（主要來源）與 BigGo（備援）。

use std::time::Duration;

use anyhow::{Context, Result};
use rand::RngExt;

use crate::{
    app::backfill::acl::{BigGoFinancialReportAclMapper, YahooFinancialReportAclMapper},
    domain::financial::statement::{BalanceSheet, CashFlowStatement, IncomeStatement},
    infra::crawler::{
        biggo::financial_statement::{self as biggo_statement, FiscalPeriod, ReportKind},
        yahoo::financial_statement::{ReportPeriod, balance_sheet, cash_flow, income_statement},
    },
};

/// 單檔三大報表轉譯後的領域實體。
pub(super) struct StockReports {
    pub(super) income_statements: Vec<IncomeStatement>,
    pub(super) balance_sheets: Vec<BalanceSheet>,
    pub(super) cash_flow_statements: Vec<CashFlowStatement>,
}

impl StockReports {
    /// 三張表都沒有任何資料列。
    pub(super) fn is_empty(&self) -> bool {
        self.income_statements.is_empty()
            && self.balance_sheets.is_empty()
            && self.cash_flow_statements.is_empty()
    }
}

/// 依序發出 5 次請求並轉譯；任一步失敗即回傳錯誤。
pub(super) async fn fetch_reports(stock_symbol: &str) -> Result<StockReports> {
    let mut income_statements = Vec::new();
    let mut cash_flow_statements = Vec::new();

    for period in [ReportPeriod::Quarter, ReportPeriod::Year] {
        let rows = income_statement::visit(stock_symbol, period)
            .await
            .with_context(|| {
                format!("fetch income statements failed: {stock_symbol} {period:?}")
            })?;
        for row in &rows {
            income_statements.push(YahooFinancialReportAclMapper::income_statement(row)?);
        }
        request_interval().await;

        let rows = cash_flow::visit(stock_symbol, period)
            .await
            .with_context(|| {
                format!("fetch cash flow statements failed: {stock_symbol} {period:?}")
            })?;
        for row in &rows {
            cash_flow_statements.push(YahooFinancialReportAclMapper::cash_flow_statement(row)?);
        }
        request_interval().await;
    }

    let balance_sheets = balance_sheet::visit(stock_symbol)
        .await
        .with_context(|| format!("fetch balance sheets failed: {stock_symbol}"))?
        .iter()
        .map(YahooFinancialReportAclMapper::balance_sheet)
        .collect::<Result<Vec<_>>>()?;

    Ok(StockReports {
        income_statements,
        balance_sheets,
        cash_flow_statements,
    })
}

/// 依序向 BigGo 發出 9 次請求並轉譯；任一步失敗即回傳錯誤。
pub(super) async fn fetch_biggo_reports(stock_symbol: &str) -> Result<StockReports> {
    let context = |what: &str| format!("fetch BigGo {what} failed: {stock_symbol}");

    let single_income = biggo_statement::income_statements(stock_symbol, ReportKind::Single)
        .await
        .with_context(|| context("single income statements"))?;
    request_interval().await;
    let annual_income = biggo_statement::income_statements(stock_symbol, ReportKind::Annual)
        .await
        .with_context(|| context("annual income statements"))?;
    request_interval().await;
    let single_cash_flow = biggo_statement::cash_flow_statements(stock_symbol, ReportKind::Single)
        .await
        .with_context(|| context("single cash flow statements"))?;
    request_interval().await;
    let annual_cash_flow = biggo_statement::cash_flow_statements(stock_symbol, ReportKind::Annual)
        .await
        .with_context(|| context("annual cash flow statements"))?;
    request_interval().await;
    let balance_sheets = biggo_statement::balance_sheets(stock_symbol)
        .await
        .with_context(|| context("balance sheets"))?;

    let income_quarters: Vec<FiscalPeriod> = single_income.iter().map(|s| s.period).collect();
    let cash_flow_quarters: Vec<FiscalPeriod> = single_cash_flow.iter().map(|s| s.period).collect();
    let annual_income = biggo_statement::retain_completed_years(annual_income, &income_quarters);
    let annual_cash_flow =
        biggo_statement::retain_completed_years(annual_cash_flow, &cash_flow_quarters);

    Ok(StockReports {
        income_statements: single_income
            .iter()
            .chain(&annual_income)
            .map(BigGoFinancialReportAclMapper::income_statement)
            .collect::<Result<_>>()?,
        balance_sheets: balance_sheets
            .iter()
            .map(BigGoFinancialReportAclMapper::balance_sheet)
            .collect::<Result<_>>()?,
        cash_flow_statements: single_cash_flow
            .iter()
            .chain(&annual_cash_flow)
            .map(BigGoFinancialReportAclMapper::cash_flow_statement)
            .collect::<Result<_>>()?,
    })
}

/// 同一檔股票的請求之間隨機停 0.3～0.8 秒。
async fn request_interval() {
    let ms = rand::rng().random_range(300..=800);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}
