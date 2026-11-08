//! BigGo 三大財務報表 DTO → 領域實體的轉譯（Yahoo 財報的備援）。
//!
//! 轉譯規則與 [`super::YahooFinancialReportAclMapper`] 一致：
//!
//! - 金額由**元**換算成**千元**，無法被 1000 整除時整筆拒收。
//! - 期別：單季 → `Single`、年度 → `Annual`；期別與季別組合不合理時拒收。
//! - `source` 寫 `biggo`，與 Yahoo 的資料列並存、不互相覆蓋。
//! - BigGo 沒有、或定義與 Yahoo 不同的欄位（銷售／管理／研發費用、稅前淨利、母公司業主淨利、
//!   每股數值、應收帳款、短期借款、保留盈餘、折舊、攤銷等）一律為 `None`。

use anyhow::{Context, Result, anyhow, bail};

use super::financial_report::to_quarter;
use crate::{
    domain::financial::statement::{
        BalanceSheet, CashFlowStatement, IncomeStatement, StatementPeriod,
    },
    infra::crawler::biggo::financial_statement::{
        BalanceItems, CashFlowItems, FiscalPeriod, IncomeItems, ReportKind, Statement,
    },
};

/// 寫入資料表 `source` 欄位的來源識別。
const SOURCE: &str = "biggo";

/// BigGo 財務報表防腐層轉譯器。
pub struct BigGoFinancialReportAclMapper;

impl BigGoFinancialReportAclMapper {
    /// 將 BigGo 損益表轉譯為領域實體。
    ///
    /// # Errors
    ///
    /// 期別組合不合理或金額無法整除 1000 時回傳錯誤。
    pub fn income_statement(dto: &Statement<IncomeItems>) -> Result<IncomeStatement> {
        let convert = || -> Result<IncomeStatement> {
            let items = &dto.items;
            Ok(IncomeStatement {
                stock_symbol: dto.stock_symbol.clone(),
                fiscal_year: dto.period.year,
                period: statement_period(dto.kind, dto.period)?,
                source: SOURCE.to_string(),
                revenue: to_thousands(items.revenue, "revenue")?,
                gross_profit: to_thousands(items.gross_profit, "gross_profit")?,
                selling_expenses: None,
                admin_expenses: None,
                rd_expenses: None,
                operating_expenses: to_thousands(items.operating_expenses, "operating_expenses")?,
                operating_profit: to_thousands(items.operating_income, "operating_income")?,
                non_operating_income: None,
                profit_before_tax: None,
                net_income: to_thousands(items.income_after_taxes, "income_after_taxes")?,
                owner_parent_profit: None,
                revenue_per_share: None,
                operating_profit_per_share: None,
                profit_before_tax_per_share: None,
                eps: items.eps,
                bps: None,
            })
        };
        convert().with_context(|| describe("income statement", dto))
    }

    /// 將 BigGo 資產負債表轉譯為領域實體。
    ///
    /// # Errors
    ///
    /// 不是單季資料或金額無法整除 1000 時回傳錯誤。
    pub fn balance_sheet(dto: &Statement<BalanceItems>) -> Result<BalanceSheet> {
        let convert = || -> Result<BalanceSheet> {
            let quarter = match statement_period(dto.kind, dto.period)? {
                StatementPeriod::Single(quarter) => quarter,
                other => bail!("balance sheet only supports single quarter, got {other:?}"),
            };
            let items = &dto.items;
            let amount = to_thousands;
            Ok(BalanceSheet {
                stock_symbol: dto.stock_symbol.clone(),
                fiscal_year: dto.period.year,
                quarter,
                source: SOURCE.to_string(),
                cash_and_equivalents: amount(
                    items.cash_and_cash_equivalents,
                    "cash_and_cash_equivalents",
                )?,
                short_term_investments: amount(
                    items.current_financial_assets_fvtpl,
                    "current_financial_assets_fvtpl",
                )?,
                accounts_receivable: None,
                inventory: amount(items.inventories, "inventories")?,
                other_current_assets: amount(items.other_current_assets, "other_current_assets")?,
                current_assets: amount(items.current_assets, "current_assets")?,
                equity_and_other_investments: amount(
                    items.investment_accounted_equity_method,
                    "investment_accounted_equity_method",
                )?,
                property_plant_equipment: amount(
                    items.property_plant_equipment,
                    "property_plant_equipment",
                )?,
                right_of_use_asset: amount(items.right_of_use_asset, "right_of_use_asset")?,
                non_current_assets: amount(items.noncurrent_assets, "noncurrent_assets")?,
                total_assets: amount(items.total_assets, "total_assets")?,
                long_term_investment: None,
                short_term_investment: None,
                other_assets: None,
                short_term_debt: None,
                short_term_bills_payable: None,
                accounts_payable: None,
                current_portion_of_long_term_liabilities: None,
                current_liabilities: amount(items.current_liabilities, "current_liabilities")?,
                long_term_liabilities: None,
                bonds_payable: amount(items.bonds_payable, "bonds_payable")?,
                other_liabilities: None,
                non_current_liabilities: amount(
                    items.noncurrent_liabilities,
                    "noncurrent_liabilities",
                )?,
                total_liabilities: amount(items.total_liabilities, "total_liabilities")?,
                share_capital: amount(items.capital_stock, "capital_stock")?,
                retained_earnings: None,
                equity: amount(items.equity, "equity")?,
                book_value_per_share: None,
            })
        };
        convert().with_context(|| describe("balance sheet", dto))
    }

    /// 將 BigGo 現金流量表轉譯為領域實體。
    ///
    /// # Errors
    ///
    /// 期別組合不合理或金額無法整除 1000 時回傳錯誤。
    pub fn cash_flow_statement(dto: &Statement<CashFlowItems>) -> Result<CashFlowStatement> {
        let convert = || -> Result<CashFlowStatement> {
            let items = &dto.items;
            Ok(CashFlowStatement {
                stock_symbol: dto.stock_symbol.clone(),
                fiscal_year: dto.period.year,
                period: statement_period(dto.kind, dto.period)?,
                source: SOURCE.to_string(),
                depreciation: None,
                amortization: None,
                operating_cash_flow: to_thousands(
                    items.operating_cash_flow,
                    "operating_cash_flow",
                )?,
                investing_cash_flow: to_thousands(
                    items.investing_cash_flow,
                    "investing_cash_flow",
                )?,
                financing_cash_flow: to_thousands(
                    items.financing_cash_flow,
                    "financing_cash_flow",
                )?,
                free_cash_flow: to_thousands(items.free_cash_flow, "free_cash_flow")?,
                net_cash_flow: to_thousands(items.net_cash_flow, "net_cash_flow")?,
            })
        };
        convert().with_context(|| describe("cash flow statement", dto))
    }
}

/// 錯誤訊息用的報表識別，例如 `BigGo income statement 2330 2026 Single Some(2)`。
fn describe<T>(kind: &str, dto: &Statement<T>) -> String {
    format!(
        "Failed to map BigGo {kind} {} {} {:?} {:?}",
        dto.stock_symbol, dto.period.year, dto.kind, dto.period.quarter
    )
}

/// 由爬蟲的期別與季別組出領域期別。
fn statement_period(kind: ReportKind, period: FiscalPeriod) -> Result<StatementPeriod> {
    match (kind, period.quarter) {
        (ReportKind::Single, Some(quarter)) => Ok(StatementPeriod::Single(to_quarter(quarter)?)),
        (ReportKind::Annual, None) => Ok(StatementPeriod::Annual),
        (kind, quarter) => Err(anyhow!(
            "inconsistent period {kind:?} with quarter {quarter:?}"
        )),
    }
}

/// 元換算千元；無法整除 1000 時回傳錯誤，`None` 維持 `None`。
fn to_thousands(value: Option<i64>, field: &str) -> Result<Option<i64>> {
    match value {
        Some(value) if value % 1000 != 0 => bail!("{field} = {value} is not a multiple of 1000"),
        Some(value) => Ok(Some(value / 1000)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
