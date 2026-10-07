//! `financial_statement` 資料表：財報模型、各來源轉換，以及寫入（[`mutation`]）與查詢（[`query`]）。

mod mutation;
mod query;

use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{Row, postgres::PgRow};

pub use self::query::{
    fetch_annual, fetch_cumulative_eps, fetch_roe_or_roa_equal_to_zero, fetch_without_annual,
};
use crate::{
    core::util::map::Keyable,
    infra::crawler::{self, twse, wespai, yahoo},
};

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone, Deserialize, Serialize)]
/// 財務報表
pub struct FinancialStatement {
    pub updated_time: DateTime<Local>,
    pub created_time: DateTime<Local>,
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: String,
    /// 股票代號。
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
    pub serial: i64,
    /// 年度
    pub year: i64,
}

impl Keyable for FinancialStatement {
    fn key(&self) -> String {
        format!("{}-{}-{}", self.security_code, self.year, self.quarter)
    }

    fn key_with_prefix(&self) -> String {
        format!("FinancialStatement:{}", self.key())
    }
}

impl FinancialStatement {
    /// 建立指定股票代號的財報模型預設值。
    pub fn new(security_code: String) -> Self {
        FinancialStatement {
            updated_time: Default::default(),
            created_time: Default::default(),
            quarter: "".to_string(),
            security_code,
            gross_profit: Default::default(),
            operating_profit_margin: Default::default(),
            pre_tax_income: Default::default(),
            net_income: Default::default(),
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

    fn row_to_entity(row: PgRow) -> std::result::Result<FinancialStatement, sqlx::Error> {
        Ok(FinancialStatement {
            updated_time: row.try_get("updated_time")?,
            created_time: row.try_get("created_time")?,
            quarter: row.try_get("quarter")?,
            security_code: row.try_get("security_code")?,
            gross_profit: row.try_get("gross_profit")?,
            operating_profit_margin: row.try_get("operating_profit_margin")?,
            pre_tax_income: row.try_get("pre-tax_income")?,
            net_income: row.try_get("net_income")?,
            net_asset_value_per_share: row.try_get("net_asset_value_per_share")?,
            sales_per_share: row.try_get("sales_per_share")?,
            earnings_per_share: row.try_get("earnings_per_share")?,
            profit_before_tax: row.try_get("profit_before_tax")?,
            return_on_equity: row.try_get("return_on_equity")?,
            return_on_assets: row.try_get("return_on_assets")?,
            serial: row.try_get("serial")?,
            year: row.try_get("year")?,
        })
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<yahoo::profile::Profile> for FinancialStatement {
    fn from(fs: yahoo::profile::Profile) -> Self {
        let mut e = FinancialStatement::new(fs.stock_symbol);
        e.updated_time = Local::now();
        e.created_time = Local::now();
        e.quarter = fs.quarter;
        e.gross_profit = fs.gross_profit;
        e.operating_profit_margin = fs.operating_profit_margin;
        e.pre_tax_income = fs.pre_tax_income;
        e.net_income = fs.net_income;
        e.net_asset_value_per_share = fs.net_asset_value_per_share;
        e.sales_per_share = fs.sales_per_share;
        e.earnings_per_share = fs.earnings_per_share;
        e.profit_before_tax = fs.profit_before_tax;
        e.return_on_equity = fs.return_on_equity;
        e.return_on_assets = fs.return_on_assets;
        e.year = fs.year as i64;
        e
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<wespai::profit::Profit> for FinancialStatement {
    fn from(fs: wespai::profit::Profit) -> Self {
        let mut e = FinancialStatement::new(fs.security_code);
        e.updated_time = Local::now();
        e.created_time = Local::now();
        e.quarter = fs.quarter;
        e.gross_profit = fs.gross_profit;
        e.operating_profit_margin = fs.operating_profit_margin;
        e.pre_tax_income = fs.pre_tax_income;
        e.net_income = fs.net_income;
        e.net_asset_value_per_share = fs.net_asset_value_per_share;
        e.sales_per_share = fs.sales_per_share;
        e.earnings_per_share = fs.earnings_per_share;
        e.profit_before_tax = fs.profit_before_tax;
        e.return_on_equity = fs.return_on_equity;
        e.return_on_assets = fs.return_on_assets;
        e.year = fs.year as i64;
        e
    }
}

impl From<twse::eps::Eps> for FinancialStatement {
    fn from(fs: twse::eps::Eps) -> Self {
        let mut e = FinancialStatement::new(fs.stock_symbol);
        e.updated_time = Local::now();
        e.created_time = Local::now();
        e.quarter = fs.quarter.to_string();
        e.gross_profit = Default::default();
        e.operating_profit_margin = Default::default();
        e.pre_tax_income = Default::default();
        e.net_income = Default::default();
        e.net_asset_value_per_share = Default::default();
        e.sales_per_share = Default::default();
        e.earnings_per_share = fs.earnings_per_share;
        e.profit_before_tax = Default::default();
        e.return_on_equity = Default::default();
        e.return_on_assets = Default::default();
        e.year = fs.year as i64;
        e
    }
}

impl From<crawler::share::AnnualProfit> for FinancialStatement {
    fn from(fs: crate::infra::crawler::share::AnnualProfit) -> Self {
        let mut e = FinancialStatement::new(fs.stock_symbol);
        e.updated_time = Local::now();
        e.created_time = Local::now();
        e.quarter = String::from("");
        e.gross_profit = Default::default();
        e.operating_profit_margin = Default::default();
        e.pre_tax_income = Default::default();
        e.net_income = Default::default();
        e.net_asset_value_per_share = Default::default();
        e.sales_per_share = fs.sales_per_share;
        e.earnings_per_share = fs.earnings_per_share;
        e.profit_before_tax = fs.profit_before_tax;
        e.return_on_equity = Default::default();
        e.return_on_assets = Default::default();
        e.year = fs.year as i64;
        e
    }
}

#[cfg(test)]
mod tests;
