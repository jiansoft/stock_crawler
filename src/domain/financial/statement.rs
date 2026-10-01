//! # 三大財務報表（損益表、資產負債表、現金流量表）
//!
//! 對應資料表 `income_statement`、`balance_sheet`、`cash_flow_statement`。
//! 與 [`super::entity::FinancialStatement`]（比率型財報摘要）不同，這裡保存的是報表**原始金額**。
//!
//! ## 單位
//!
//! - 金額一律為新台幣**千元**（`i64`）；來源的元換算在防腐層進行，無法整除 1000 時拒收。
//! - 每股數值為新台幣元（`Decimal`，資料庫 `numeric(12, 2)`）。
//! - `None` 表示來源未提供，**不可**當成 0。

use rust_decimal::Decimal;

use crate::core::declare::Quarter;

/// 報表期間口徑。
///
/// 資料庫以 `period_type` + `quarter` 兩欄表示，對應關係固定：
///
/// | 變體 | `period_type` | `quarter` |
/// |------|---------------|-----------|
/// | `Single(q)` | `single` | `Q1`～`Q4` |
/// | `Cumulative(q)` | `cumulative` | `Q1`～`Q4` |
/// | `Annual` | `annual` | `A` |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementPeriod {
    /// 單季。
    Single(Quarter),
    /// 年初累計至該季。目前採集不寫入（可由單季加總推得），保留供日後使用。
    Cumulative(Quarter),
    /// 全年度。
    Annual,
}

impl StatementPeriod {
    /// 資料庫 `period_type` 欄位值。
    pub fn period_type(self) -> &'static str {
        match self {
            Self::Single(_) => "single",
            Self::Cumulative(_) => "cumulative",
            Self::Annual => "annual",
        }
    }

    /// 資料庫 `quarter` 欄位值；全年度為 `A`（與 `financial_statement`、`dividend` 一致）。
    pub fn quarter_code(self) -> &'static str {
        match self {
            Self::Single(quarter) | Self::Cumulative(quarter) => quarter_code(quarter),
            Self::Annual => "A",
        }
    }
}

/// 季別的資料庫字串。
fn quarter_code(quarter: Quarter) -> &'static str {
    match quarter {
        Quarter::Q1 => "Q1",
        Quarter::Q2 => "Q2",
        Quarter::Q3 => "Q3",
        Quarter::Q4 => "Q4",
    }
}

/// 損益表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeStatement {
    /// 股票代號（不含市場後綴）。
    pub stock_symbol: String,
    /// 財報所屬年度。
    pub fiscal_year: i32,
    /// 期間口徑。
    pub period: StatementPeriod,
    /// 資料來源識別，例如 `yahoo`。
    pub source: String,
    /// 營業收入（千元）。
    pub revenue: Option<i64>,
    /// 營業毛利（千元）。
    pub gross_profit: Option<i64>,
    /// 推銷費用（千元）。
    pub selling_expenses: Option<i64>,
    /// 管理費用（千元）。
    pub admin_expenses: Option<i64>,
    /// 研究發展費用（千元）。
    pub rd_expenses: Option<i64>,
    /// 營業費用（千元）。
    pub operating_expenses: Option<i64>,
    /// 營業利益（千元）。
    pub operating_profit: Option<i64>,
    /// 營業外收入及支出（千元）。
    pub non_operating_income: Option<i64>,
    /// 稅前淨利（千元）。
    pub profit_before_tax: Option<i64>,
    /// 本期稅後淨利，含非控制權益（千元）。
    pub net_income: Option<i64>,
    /// 歸屬母公司業主淨利（千元）。
    pub owner_parent_profit: Option<i64>,
    /// 每股營收（元）。
    pub revenue_per_share: Option<Decimal>,
    /// 每股營業利益（元）。
    pub operating_profit_per_share: Option<Decimal>,
    /// 每股稅前淨利（元）。
    pub profit_before_tax_per_share: Option<Decimal>,
    /// 每股盈餘（元）；年度與累季為來源官方值，不等於各單季相加。
    pub eps: Option<Decimal>,
    /// 每股淨值（元）；期末時點數值。
    pub bps: Option<Decimal>,
}

/// 資產負債表（季末時點數值）。
///
/// 只有單季：全年度即為第 4 季，因此以 [`Quarter`] 而非 [`StatementPeriod`] 表示期間。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceSheet {
    /// 股票代號（不含市場後綴）。
    pub stock_symbol: String,
    /// 財報所屬年度。
    pub fiscal_year: i32,
    /// 季別。
    pub quarter: Quarter,
    /// 資料來源識別，例如 `yahoo`。
    pub source: String,
    /// 現金及約當現金（千元）。
    pub cash_and_equivalents: Option<i64>,
    /// 短期投資，來源 `shortTermInvestments`（千元）；與 `short_term_investment` 不同欄。
    pub short_term_investments: Option<i64>,
    /// 應收帳款及票據（千元）。
    pub accounts_receivable: Option<i64>,
    /// 存貨（千元）。
    pub inventory: Option<i64>,
    /// 其他流動資產（千元）。
    pub other_current_assets: Option<i64>,
    /// 流動資產（千元）。
    pub current_assets: Option<i64>,
    /// 權益法及其他投資（千元）。
    pub equity_and_other_investments: Option<i64>,
    /// 不動產、廠房及設備（千元）。
    pub property_plant_equipment: Option<i64>,
    /// 使用權資產（千元）。
    pub right_of_use_asset: Option<i64>,
    /// 非流動資產（千元）。
    pub non_current_assets: Option<i64>,
    /// 資產總額（千元）。
    pub total_assets: Option<i64>,
    /// 長期投資（千元）。
    pub long_term_investment: Option<i64>,
    /// 短期投資，來源 `shortTermInvestment`（千元）；與 `short_term_investments` 不同欄。
    pub short_term_investment: Option<i64>,
    /// 其他資產（千元）。
    pub other_assets: Option<i64>,
    /// 短期借款（千元）。
    pub short_term_debt: Option<i64>,
    /// 應付短期票券（千元）。
    pub short_term_bills_payable: Option<i64>,
    /// 應付帳款及票據（千元）。
    pub accounts_payable: Option<i64>,
    /// 一年內到期長期負債（千元）。
    pub current_portion_of_long_term_liabilities: Option<i64>,
    /// 流動負債（千元）。
    pub current_liabilities: Option<i64>,
    /// 長期負債（千元）。
    pub long_term_liabilities: Option<i64>,
    /// 應付公司債（千元）。
    pub bonds_payable: Option<i64>,
    /// 其他負債（千元）。
    pub other_liabilities: Option<i64>,
    /// 非流動負債（千元）。
    pub non_current_liabilities: Option<i64>,
    /// 負債總額（千元）。
    pub total_liabilities: Option<i64>,
    /// 股本（千元）。
    pub share_capital: Option<i64>,
    /// 保留盈餘（千元）。
    pub retained_earnings: Option<i64>,
    /// 權益總額（千元）。
    pub equity: Option<i64>,
    /// 每股淨值（元）。
    pub book_value_per_share: Option<Decimal>,
}

/// 現金流量表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashFlowStatement {
    /// 股票代號（不含市場後綴）。
    pub stock_symbol: String,
    /// 財報所屬年度。
    pub fiscal_year: i32,
    /// 期間口徑。
    pub period: StatementPeriod,
    /// 資料來源識別，例如 `yahoo`。
    pub source: String,
    /// 折舊（千元）。
    pub depreciation: Option<i64>,
    /// 攤銷（千元）。
    pub amortization: Option<i64>,
    /// 營業活動現金流量（千元）。
    pub operating_cash_flow: Option<i64>,
    /// 投資活動現金流量（千元）。
    pub investing_cash_flow: Option<i64>,
    /// 籌資活動現金流量（千元）。
    pub financing_cash_flow: Option<i64>,
    /// 自由現金流量，來源定義為營業＋投資（千元）。
    pub free_cash_flow: Option<i64>,
    /// 現金及約當現金淨增減（千元）；不可用三項現金流相加取代。
    pub net_cash_flow: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statement_period_db_codes() {
        let single = StatementPeriod::Single(Quarter::Q2);
        assert_eq!(
            (single.period_type(), single.quarter_code()),
            ("single", "Q2")
        );

        let cumulative = StatementPeriod::Cumulative(Quarter::Q4);
        assert_eq!(
            (cumulative.period_type(), cumulative.quarter_code()),
            ("cumulative", "Q4")
        );

        let annual = StatementPeriod::Annual;
        assert_eq!(
            (annual.period_type(), annual.quarter_code()),
            ("annual", "A")
        );
    }
}
