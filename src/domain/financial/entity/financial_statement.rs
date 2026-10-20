//! 財務報表實體：特定年度與季度的毛利率、營業利益率、EPS 等。

use chrono::{DateTime, Local};
use rust_decimal::Decimal;

/// 財務報表領域實體。
///
/// 封裝公司在特定年度與季度的財務數據（如毛利率、營業利益率、EPS 等）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialStatement {
    /// 序號
    pub serial: i64,
    /// 股票代號
    pub security_code: String,
    /// 年度
    pub year: i64,
    /// 季度 (Q1, Q2, Q3, Q4)
    pub quarter: String,
    /// 營業毛利率 (%)
    pub gross_profit: Decimal,
    /// 營業利益率 (%)
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率 (%)
    pub pre_tax_income: Decimal,
    /// 稅後淨利率 (%)
    pub net_income: Decimal,
    /// 每股淨值 (元)
    pub net_asset_value_per_share: Decimal,
    /// 每股營收 (元)
    pub sales_per_share: Decimal,
    /// 每股稅後淨利 (EPS, 元)
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利 (元)
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率 (ROE, %)
    pub return_on_equity: Decimal,
    /// 資產報酬率 (ROA, %)
    pub return_on_assets: Decimal,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}

impl Default for FinancialStatement {
    fn default() -> Self {
        FinancialStatement {
            serial: 0,
            security_code: String::new(),
            year: 0,
            quarter: String::new(),
            gross_profit: Decimal::ZERO,
            operating_profit_margin: Decimal::ZERO,
            pre_tax_income: Decimal::ZERO,
            net_income: Decimal::ZERO,
            net_asset_value_per_share: Decimal::ZERO,
            sales_per_share: Decimal::ZERO,
            earnings_per_share: Decimal::ZERO,
            profit_before_tax: Decimal::ZERO,
            return_on_equity: Decimal::ZERO,
            return_on_assets: Decimal::ZERO,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

impl FinancialStatement {
    /// 建立指定股票代號的財報模型。
    pub fn new(security_code: String) -> Self {
        FinancialStatement {
            security_code,
            ..Default::default()
        }
    }
}
