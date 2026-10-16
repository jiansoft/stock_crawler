//! 期別解析與各 endpoint 依期別合併成 [`Statement`]。

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use rust_decimal::{Decimal, prelude::FromPrimitive};

use super::rows::{AssetsRow, CashFlowRow, EpsRow, EquityRow, LiabilitiesRow, ProfitRow};
use super::{BalanceItems, CashFlowItems, FiscalPeriod, IncomeItems, ReportKind, Statement};

/// 解析 `period`：單季 `"2026 Q2"`、年度 `"2026"`，其餘一律視為異常。
pub(super) fn parse_period(period: &str, kind: ReportKind) -> Result<FiscalPeriod> {
    let invalid = || anyhow!("Unexpected BigGo period {period:?} for {kind:?}");
    let parse_year = |year: &str| -> Result<i32> {
        if year.len() != 4 {
            return Err(invalid());
        }
        year.parse().map_err(|_| invalid())
    };
    match kind {
        ReportKind::Single => {
            let (year, quarter) = period.split_once(" Q").ok_or_else(invalid)?;
            let quarter: u8 = quarter.parse().map_err(|_| invalid())?;
            if !(1..=4).contains(&quarter) {
                return Err(invalid());
            }
            Ok(FiscalPeriod {
                year: parse_year(year)?,
                quarter: Some(quarter),
            })
        }
        ReportKind::Annual => Ok(FiscalPeriod {
            year: parse_year(period)?,
            quarter: None,
        }),
    }
}

/// 依期別合併損益與 EPS；任一邊缺某期時該期仍保留，缺的項目為 `None`。
pub(super) fn merge_income(
    stock_symbol: &str,
    kind: ReportKind,
    profit: Vec<ProfitRow>,
    eps: Vec<EpsRow>,
) -> Result<Vec<Statement<IncomeItems>>> {
    let mut merged: BTreeMap<FiscalPeriod, IncomeItems> = BTreeMap::new();
    for row in profit {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.revenue = row.revenue;
        items.gross_profit = row.gross_profit;
        items.operating_expenses = row.operating_expenses;
        items.operating_income = row.operating_income;
        items.income_after_taxes = row.income_after_taxes;
    }
    for row in eps {
        let period = parse_period(&row.period, kind)?;
        let eps = row
            .eps
            .map(|value| {
                Decimal::from_f64(value)
                    .map(|value| value.round_dp(4))
                    .ok_or_else(|| anyhow!("EPS {value} for {} is out of range", row.period))
            })
            .transpose()?;
        merged.entry(period).or_default().eps = eps;
    }
    Ok(into_statements(stock_symbol, kind, merged))
}

/// 解析現金流量表。
pub(super) fn parse_cash_flow(
    stock_symbol: &str,
    kind: ReportKind,
    rows: Vec<CashFlowRow>,
) -> Result<Vec<Statement<CashFlowItems>>> {
    let mut merged = BTreeMap::new();
    for row in rows {
        merged.insert(
            parse_period(&row.period, kind)?,
            CashFlowItems {
                operating_cash_flow: row.operating_cash_flow,
                investing_cash_flow: row.investing_cash_flow,
                financing_cash_flow: row.financing_cash_flow,
                free_cash_flow: row.free_cash_flow,
                net_cash_flow: row.net_cash_flow,
            },
        );
    }
    Ok(into_statements(stock_symbol, kind, merged))
}

/// 依期別合併資產、負債、權益三份明細。
pub(super) fn merge_balance(
    stock_symbol: &str,
    assets: Vec<AssetsRow>,
    liabilities: Vec<LiabilitiesRow>,
    equity: Vec<EquityRow>,
) -> Result<Vec<Statement<BalanceItems>>> {
    let kind = ReportKind::Single;
    let mut merged: BTreeMap<FiscalPeriod, BalanceItems> = BTreeMap::new();
    for row in assets {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.cash_and_cash_equivalents = row.cash_and_cash_equivalents;
        items.current_financial_assets_fvtpl = row.current_financial_assets_fvtpl;
        items.inventories = row.inventories;
        items.other_current_assets = row.other_current_assets;
        items.current_assets = row.current_assets;
        items.investment_accounted_equity_method = row.investment_accounted_equity_method;
        items.property_plant_equipment = row.property_plant_equipment;
        items.right_of_use_asset = row.right_of_use_asset;
        items.noncurrent_assets = row.noncurrent_assets;
        items.total_assets = row.total_assets;
    }
    for row in liabilities {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.current_liabilities = row.current_liabilities;
        items.bonds_payable = row.bonds_payable;
        items.noncurrent_liabilities = row.noncurrent_liabilities;
        items.total_liabilities = row.total_liabilities;
    }
    for row in equity {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.capital_stock = row.capital_stock;
        items.equity = row.equity;
    }
    Ok(into_statements(stock_symbol, kind, merged))
}

/// 把依期別合併好的項目轉成 [`Statement`]，期別由舊到新。
fn into_statements<T>(
    stock_symbol: &str,
    kind: ReportKind,
    merged: BTreeMap<FiscalPeriod, T>,
) -> Vec<Statement<T>> {
    merged
        .into_iter()
        .map(|(period, items)| Statement {
            stock_symbol: stock_symbol.to_string(),
            kind,
            period,
            items,
        })
        .collect()
}
