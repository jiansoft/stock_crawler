//! 單檔三大報表的抓取與轉譯：Yahoo（主要來源）與 BigGo（備援）。
//!
//! 每個來源分成兩段：`fetch_*` 只負責依序發出請求並收集原始資料，
//! `assemble_*` 是不碰網路的純函式，負責篩選期別並轉譯成領域實體。

use std::time::Duration;

use anyhow::{Context, Result};
use rand::RngExt;

use crate::{
    app::backfill::acl::{BigGoFinancialReportAclMapper, YahooFinancialReportAclMapper},
    domain::financial::statement::{BalanceSheet, CashFlowStatement, IncomeStatement},
    infra::crawler::{
        biggo::financial_statement::{
            self as biggo_statement, BalanceItems, CashFlowItems as BigGoCashFlowItems,
            FiscalPeriod, IncomeItems, ReportKind, Statement,
        },
        yahoo::financial_statement::{
            FinancialStatement, ReportPeriod,
            balance_sheet::{self, BalanceSheetItems},
            cash_flow::{self, CashFlowItems},
            income_statement::{self, IncomeStatementItems},
        },
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

/// Yahoo 單檔 5 次請求的原始資料。
#[derive(Default)]
struct YahooRaw {
    /// 單季＋年度損益表。
    income_statements: Vec<FinancialStatement<IncomeStatementItems>>,
    /// 單季＋年度現金流量表。
    cash_flow_statements: Vec<FinancialStatement<CashFlowItems>>,
    /// 單季資產負債表。
    balance_sheets: Vec<FinancialStatement<BalanceSheetItems>>,
}

/// 依序向 Yahoo 發出 5 次請求並轉譯；任一步失敗即回傳錯誤。
pub(super) async fn fetch_reports(stock_symbol: &str) -> Result<StockReports> {
    let mut raw = YahooRaw::default();

    for period in [ReportPeriod::Quarter, ReportPeriod::Year] {
        raw.income_statements.extend(
            income_statement::visit(stock_symbol, period)
                .await
                .with_context(|| {
                    format!("fetch income statements failed: {stock_symbol} {period:?}")
                })?,
        );
        request_interval().await;

        raw.cash_flow_statements.extend(
            cash_flow::visit(stock_symbol, period)
                .await
                .with_context(|| {
                    format!("fetch cash flow statements failed: {stock_symbol} {period:?}")
                })?,
        );
        request_interval().await;
    }

    raw.balance_sheets = balance_sheet::visit(stock_symbol)
        .await
        .with_context(|| format!("fetch balance sheets failed: {stock_symbol}"))?;

    assemble_yahoo(&raw)
}

/// 把 Yahoo 原始資料轉譯成領域實體；任一列轉譯失敗（如金額無法整除 1000）即回傳錯誤。
fn assemble_yahoo(raw: &YahooRaw) -> Result<StockReports> {
    Ok(StockReports {
        income_statements: raw
            .income_statements
            .iter()
            .map(YahooFinancialReportAclMapper::income_statement)
            .collect::<Result<_>>()?,
        balance_sheets: raw
            .balance_sheets
            .iter()
            .map(YahooFinancialReportAclMapper::balance_sheet)
            .collect::<Result<_>>()?,
        cash_flow_statements: raw
            .cash_flow_statements
            .iter()
            .map(YahooFinancialReportAclMapper::cash_flow_statement)
            .collect::<Result<_>>()?,
    })
}

/// BigGo 單檔 9 次請求的原始資料。
struct BigGoRaw {
    single_income: Vec<Statement<IncomeItems>>,
    annual_income: Vec<Statement<IncomeItems>>,
    single_cash_flow: Vec<Statement<BigGoCashFlowItems>>,
    annual_cash_flow: Vec<Statement<BigGoCashFlowItems>>,
    balance_sheets: Vec<Statement<BalanceItems>>,
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

    assemble_biggo(BigGoRaw {
        single_income,
        annual_income,
        single_cash_flow,
        annual_cash_flow,
        balance_sheets,
    })
}

/// 把 BigGo 原始資料轉譯成領域實體。
///
/// BigGo 的年度資料含當年度至今的累計，只保留已有第四季單季資料的年度。
fn assemble_biggo(raw: BigGoRaw) -> Result<StockReports> {
    let income_quarters: Vec<FiscalPeriod> = raw.single_income.iter().map(|s| s.period).collect();
    let cash_flow_quarters: Vec<FiscalPeriod> =
        raw.single_cash_flow.iter().map(|s| s.period).collect();
    let annual_income =
        biggo_statement::retain_completed_years(raw.annual_income, &income_quarters);
    let annual_cash_flow =
        biggo_statement::retain_completed_years(raw.annual_cash_flow, &cash_flow_quarters);

    Ok(StockReports {
        income_statements: raw
            .single_income
            .iter()
            .chain(&annual_income)
            .map(BigGoFinancialReportAclMapper::income_statement)
            .collect::<Result<_>>()?,
        balance_sheets: raw
            .balance_sheets
            .iter()
            .map(BigGoFinancialReportAclMapper::balance_sheet)
            .collect::<Result<_>>()?,
        cash_flow_statements: raw
            .single_cash_flow
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

#[cfg(test)]
mod tests;
