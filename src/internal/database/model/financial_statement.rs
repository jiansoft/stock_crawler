use crate::{
    internal::{
        crawler::yahoo,
        database::DB
    }
};
use anyhow::*;
use chrono::{DateTime, Local};
use core::result::Result::Ok;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

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

    pub async fn upsert(&self) -> Result<()> {
        let sql = r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, gross_profit, operating_profit_margin,
    "pre-tax_income", net_income, net_asset_value_per_share, sales_per_share,
    earnings_per_share, profit_before_tax, return_on_equity, return_on_assets,
    created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
ON CONFLICT (security_code,"year",quarter) DO UPDATE SET
    gross_profit = EXCLUDED.gross_profit,
    operating_profit_margin = EXCLUDED.operating_profit_margin,
    "pre-tax_income" = EXCLUDED."pre-tax_income",
    net_income = EXCLUDED.net_income,
    net_asset_value_per_share = EXCLUDED.net_asset_value_per_share,
    sales_per_share = EXCLUDED.sales_per_share,
    earnings_per_share = EXCLUDED.earnings_per_share,
    profit_before_tax = EXCLUDED.profit_before_tax,
    return_on_equity = EXCLUDED.return_on_equity,
    return_on_assets = EXCLUDED.return_on_assets,
    updated_time = EXCLUDED.updated_time;
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.gross_profit)
            .bind(self.operating_profit_margin)
            .bind(self.pre_tax_income)
            .bind(self.net_income)
            .bind(self.net_asset_value_per_share)
            .bind(self.sales_per_share)
            .bind(self.earnings_per_share)
            .bind(self.profit_before_tax)
            .bind(self.return_on_equity)
            .bind(self.return_on_assets)
            .bind(self.created_time)
            .bind(self.updated_time)
            .execute(&DB.pool)
            .await
            .map_err(|err| anyhow!("Failed to financial_statement upsert because: {:?}", err))?;

        Ok(())
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<yahoo::profile::FinancialStatement> for Entity {
    fn from(fs: yahoo::profile::FinancialStatement) -> Self {
        let mut e = Entity::new(fs.security_code);
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
        e.year = fs.year;
        e
    }
}
