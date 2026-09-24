//! Yahoo 三大財務報表 DTO → 領域實體的轉譯。
//!
//! 轉譯規則：
//!
//! - 金額由**元**換算成**千元**。無法被 1000 整除時整筆拒收，不做四捨五入——
//!   實測近 2.9 萬筆金額全為 1000 的倍數，出現例外代表來源格式變了，應該停下來看。
//! - 期別：單季 → `Single`、累計 → `Cumulative`、年度 → `Annual`；
//!   期別與季別組合不合理（年度帶季別、單季沒有季別）時拒收。
//! - 捨棄的來源欄位：損益表的 `debtCost`（語意未驗證）與 `netIncomeAcc4Q`
//!   （單位為百萬元且可由單季加總推得）。

use anyhow::{Context, Result, anyhow, bail};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    core::declare::Quarter,
    domain::financial::statement::{
        BalanceSheet, CashFlowStatement, IncomeStatement, StatementPeriod,
    },
    infra::crawler::yahoo::financial_statement::{
        FinancialStatement, FiscalPeriod, ReportPeriod, balance_sheet::BalanceSheetItems,
        cash_flow::CashFlowItems, income_statement::IncomeStatementItems,
    },
};

/// 寫入資料表 `source` 欄位的來源識別。
const SOURCE: &str = "yahoo";

/// 元換算千元的除數。
const THOUSAND: Decimal = Decimal::from_parts(1000, 0, 0, false, 0);

/// Yahoo 財務報表防腐層轉譯器。
pub struct YahooFinancialReportAclMapper;

impl YahooFinancialReportAclMapper {
    /// 將 Yahoo 損益表轉譯為領域實體。
    ///
    /// # Errors
    ///
    /// 期別組合不合理、金額無法整除 1000 或超出 `i64` 範圍時回傳錯誤。
    pub fn income_statement(
        dto: &FinancialStatement<IncomeStatementItems>,
    ) -> Result<IncomeStatement> {
        let convert = || -> Result<IncomeStatement> {
            let items = &dto.items;
            Ok(IncomeStatement {
                stock_symbol: dto.stock_symbol.clone(),
                fiscal_year: dto.fiscal_period.year,
                period: statement_period(dto.report_period, dto.fiscal_period)?,
                source: SOURCE.to_string(),
                revenue: to_thousands(items.revenue, "revenue")?,
                gross_profit: to_thousands(items.gross_profit, "gross_profit")?,
                selling_expenses: to_thousands(items.selling_expenses, "selling_expenses")?,
                admin_expenses: to_thousands(items.admin_expenses, "admin_expenses")?,
                rd_expenses: to_thousands(items.rd_expenses, "rd_expenses")?,
                operating_expenses: to_thousands(items.operating_expenses, "operating_expenses")?,
                operating_profit: to_thousands(items.operating_profit, "operating_profit")?,
                non_operating_income: to_thousands(
                    items.non_operating_income,
                    "non_operating_income",
                )?,
                profit_before_tax: to_thousands(items.profit_before_tax, "profit_before_tax")?,
                net_income: to_thousands(items.net_income, "net_income")?,
                owner_parent_profit: to_thousands(
                    items.owner_parent_profit,
                    "owner_parent_profit",
                )?,
                revenue_per_share: items.revenue_per_share,
                operating_profit_per_share: items.operating_profit_per_share,
                profit_before_tax_per_share: items.profit_before_tax_per_share,
                eps: items.eps,
                bps: items.bps,
            })
        };

        convert().with_context(|| describe("income statement", dto))
    }

    /// 將 Yahoo 資產負債表轉譯為領域實體。
    ///
    /// # Errors
    ///
    /// 不是單季資料、金額無法整除 1000 或超出 `i64` 範圍時回傳錯誤。
    pub fn balance_sheet(dto: &FinancialStatement<BalanceSheetItems>) -> Result<BalanceSheet> {
        let convert = || -> Result<BalanceSheet> {
            let quarter = match statement_period(dto.report_period, dto.fiscal_period)? {
                StatementPeriod::Single(quarter) => quarter,
                other => bail!("balance sheet only supports single quarter, got {other:?}"),
            };
            let items = &dto.items;
            let amount = to_thousands;

            Ok(BalanceSheet {
                stock_symbol: dto.stock_symbol.clone(),
                fiscal_year: dto.fiscal_period.year,
                quarter,
                source: SOURCE.to_string(),
                cash_and_equivalents: amount(items.cash_and_equivalents, "cash_and_equivalents")?,
                short_term_investments: amount(
                    items.short_term_investments,
                    "short_term_investments",
                )?,
                accounts_receivable: amount(items.accounts_receivable, "accounts_receivable")?,
                inventory: amount(items.inventory, "inventory")?,
                other_current_assets: amount(items.other_current_assets, "other_current_assets")?,
                current_assets: amount(items.current_assets, "current_assets")?,
                equity_and_other_investments: amount(
                    items.equity_and_other_investments,
                    "equity_and_other_investments",
                )?,
                property_plant_equipment: amount(
                    items.property_plant_equipment,
                    "property_plant_equipment",
                )?,
                right_of_use_asset: amount(items.right_of_use_asset, "right_of_use_asset")?,
                non_current_assets: amount(items.non_current_assets, "non_current_assets")?,
                total_assets: amount(items.total_assets, "total_assets")?,
                long_term_investment: amount(items.long_term_investment, "long_term_investment")?,
                short_term_investment: amount(
                    items.short_term_investment,
                    "short_term_investment",
                )?,
                other_assets: amount(items.other_assets, "other_assets")?,
                short_term_debt: amount(items.short_term_debt, "short_term_debt")?,
                short_term_bills_payable: amount(
                    items.short_term_bills_payable,
                    "short_term_bills_payable",
                )?,
                accounts_payable: amount(items.accounts_payable, "accounts_payable")?,
                current_portion_of_long_term_liabilities: amount(
                    items.current_portion_of_long_term_liabilities,
                    "current_portion_of_long_term_liabilities",
                )?,
                current_liabilities: amount(items.current_liabilities, "current_liabilities")?,
                long_term_liabilities: amount(
                    items.long_term_liabilities,
                    "long_term_liabilities",
                )?,
                bonds_payable: amount(items.bonds_payable, "bonds_payable")?,
                other_liabilities: amount(items.other_liabilities, "other_liabilities")?,
                non_current_liabilities: amount(
                    items.non_current_liabilities,
                    "non_current_liabilities",
                )?,
                total_liabilities: amount(items.total_liabilities, "total_liabilities")?,
                share_capital: amount(items.share_capital, "share_capital")?,
                retained_earnings: amount(items.retained_earnings, "retained_earnings")?,
                equity: amount(items.equity, "equity")?,
                book_value_per_share: items.net_worth,
            })
        };

        convert().with_context(|| describe("balance sheet", dto))
    }

    /// 將 Yahoo 現金流量表轉譯為領域實體。
    ///
    /// # Errors
    ///
    /// 期別組合不合理、金額無法整除 1000 或超出 `i64` 範圍時回傳錯誤。
    pub fn cash_flow_statement(
        dto: &FinancialStatement<CashFlowItems>,
    ) -> Result<CashFlowStatement> {
        let convert = || -> Result<CashFlowStatement> {
            let items = &dto.items;
            Ok(CashFlowStatement {
                stock_symbol: dto.stock_symbol.clone(),
                fiscal_year: dto.fiscal_period.year,
                period: statement_period(dto.report_period, dto.fiscal_period)?,
                source: SOURCE.to_string(),
                depreciation: to_thousands(items.depreciation, "depreciation")?,
                amortization: to_thousands(items.amortization, "amortization")?,
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

/// 錯誤訊息用的報表識別，例如 `Yahoo income statement 8042 2026 Quarter Some(2)`。
fn describe<T>(kind: &str, dto: &FinancialStatement<T>) -> String {
    format!(
        "Failed to map Yahoo {kind} {} {} {:?} {:?}",
        dto.stock_symbol, dto.fiscal_period.year, dto.report_period, dto.fiscal_period.quarter
    )
}

/// 由爬蟲的期別與季別組出領域期別。
fn statement_period(report_period: ReportPeriod, fiscal: FiscalPeriod) -> Result<StatementPeriod> {
    match (report_period, fiscal.quarter) {
        (ReportPeriod::Quarter, Some(quarter)) => Ok(StatementPeriod::Single(to_quarter(quarter)?)),
        (ReportPeriod::CumulativeQuarter, Some(quarter)) => {
            Ok(StatementPeriod::Cumulative(to_quarter(quarter)?))
        }
        (ReportPeriod::Year, None) => Ok(StatementPeriod::Annual),
        (period, quarter) => Err(anyhow!(
            "inconsistent period {period:?} with quarter {quarter:?}"
        )),
    }
}

/// 季別數字 1～4 轉 [`Quarter`]。
fn to_quarter(quarter: u8) -> Result<Quarter> {
    match quarter {
        1 => Ok(Quarter::Q1),
        2 => Ok(Quarter::Q2),
        3 => Ok(Quarter::Q3),
        4 => Ok(Quarter::Q4),
        other => Err(anyhow!("invalid quarter {other}")),
    }
}

/// 元換算千元；無法整除 1000 或超出 `i64` 範圍時回傳錯誤，`None` 維持 `None`。
fn to_thousands(value: Option<Decimal>, field: &str) -> Result<Option<i64>> {
    let Some(value) = value else {
        return Ok(None);
    };

    if !(value % THOUSAND).is_zero() {
        bail!("{field} = {value} is not a multiple of 1000");
    }

    (value / THOUSAND)
        .to_i64()
        .map(Some)
        .ok_or_else(|| anyhow!("{field} = {value} is out of i64 range"))
}

#[cfg(test)]
mod tests {
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

        let sheet = YahooFinancialReportAclMapper::balance_sheet(&dto(
            ReportPeriod::Quarter,
            Some(4),
            items,
        ))
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
}
