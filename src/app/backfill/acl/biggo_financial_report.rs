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
mod tests {
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

        let annual_with_quarter =
            statement(ReportKind::Annual, 2025, Some(4), IncomeItems::default());
        assert!(BigGoFinancialReportAclMapper::income_statement(&annual_with_quarter).is_err());

        let annual_balance = statement(ReportKind::Annual, 2025, None, BalanceItems::default());
        assert!(BigGoFinancialReportAclMapper::balance_sheet(&annual_balance).is_err());
    }
}
