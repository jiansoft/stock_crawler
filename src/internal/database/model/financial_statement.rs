use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use crate::internal::crawler::financial_statement::yahoo;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Entity {
    updated_time: DateTime<Local>,
    created_time: DateTime<Local>,
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: String,
    pub security_code: String,
    /// 營業毛利率
    pub gross_profit: Decimal,
    /// 營業利益率
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率
    pub pre_tax_income: Decimal,
    /// 稅後淨利率
    pub net_income: Decimal,
    /// 每股淨值
    pub net_asset_value_per_share: Decimal,
    /// 每股營收
    pub sales_per_share: Decimal,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率
    pub return_on_equity: Decimal,
    /// 資產報酬率
    pub return_on_assets: Decimal,
    serial: i64,
    /// 年度
    pub year: i32,
}

impl Entity {
    pub fn new(security_code: String) -> Self {
        Entity {
            updated_time: Default::default(),
            created_time: Default::default(),
            quarter: "".to_string(),
            security_code,
            gross_profit: Default::default(),
            operating_profit_margin: Default::default(),
            pre_tax_income: Default::default(),
            net_income:Default::default(),
            net_asset_value_per_share: Default::default(),
            sales_per_share: Default::default(),
            earnings_per_share: Default::default(),
            profit_before_tax: Default::default(),
            return_on_equity: Default::default(),
            return_on_assets: Default::default(),
            serial: 0,
            year: 0,
        }
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<yahoo::profile::FinancialStatement> for Entity {
    fn from(fs: yahoo::profile::FinancialStatement) -> Self {
        Entity {
            updated_time: Local::now(),
            created_time: Local::now(),
            quarter: fs.quarter,
            security_code: fs.security_code,
            gross_profit: fs.gross_profit,
            operating_profit_margin: fs.operating_profit_margin,
            pre_tax_income: fs.pre_tax_income,
            net_income: fs.net_income,
            net_asset_value_per_share: fs.net_asset_value_per_share,
            sales_per_share: fs.sales_per_share,
            earnings_per_share: fs.earnings_per_share,
            profit_before_tax: fs.profit_before_tax,
            return_on_equity: fs.return_on_equity,
            return_on_assets: fs.return_on_assets,
            serial: fs.serial,
            year: fs.year,
        }
    }
}
