use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{
    postgres::{PgQueryResult, PgRow},
    Row,
};

use crate::{
    crawler::{self, twse, wespai, yahoo},
    database,
    declare::Quarter,
    util::map::Keyable,
};

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone, Deserialize, Serialize)]
/// 財務報表
pub struct FinancialStatement {
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
    pub year: i64,
}

impl Keyable for FinancialStatement {
    fn key(&self) -> String {
        format!("{}-{}-{}", &self.security_code, self.year, self.quarter)
    }

    fn key_with_prefix(&self) -> String {
        format!("FinancialStatement:{}", &self.key())
    }
}

impl FinancialStatement {
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

    pub async fn upsert(self) -> Result<PgQueryResult> {
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
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    &sql,
                    why
                )
            })
    }

    pub async fn upsert_earnings_per_share(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, earnings_per_share, created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (security_code,"year",quarter) DO NOTHING;
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.earnings_per_share)
            .bind(self.created_time)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert_earnings_per_share({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    &sql,
                    why
                )
            })
    }

    pub async fn upsert_annual_eps(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO financial_statement (
    security_code, "year", quarter, earnings_per_share, profit_before_tax, sales_per_share, created_time, updated_time)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (security_code,"year",quarter) DO NOTHING;
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.earnings_per_share)
            .bind(self.profit_before_tax)
            .bind(self.sales_per_share)
            .bind(self.created_time)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert_annual_eps({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    &sql,
                    why
                )
            })
    }

    pub async fn update_roe_roa(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE
    financial_statement
SET
    return_on_equity = $4, return_on_assets = $5, updated_time = $6
WHERE
    security_code = $1 AND "year" = $2 AND quarter = $3
"#;
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(&self.quarter)
            .bind(self.return_on_equity)
            .bind(self.return_on_assets)
            .bind(self.updated_time)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to update_roe_roa({:#?}) from database\nsql:{}\n {:?}",
                    self,
                    &sql,
                    why
                )
            })
    }
}

/// 取得年度財報
pub async fn fetch_annual(year: i32) -> Result<Vec<FinancialStatement>> {
    let sql = r#"
SELECT
    serial,
    security_code,
    year,
    quarter,
    gross_profit,
    operating_profit_margin,
    "pre-tax_income",
    net_income,
    net_asset_value_per_share,
    sales_per_share,
    earnings_per_share,
    profit_before_tax,
    return_on_equity,
    return_on_assets,
    created_time,
    updated_time
FROM financial_statement
WHERE "year" = $1 AND quarter= ''
"#;
    let result = sqlx::query(sql)
        .bind(year)
        .try_map(|row: PgRow| {
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
        })
        .fetch_all(database::get_connection())
        .await?;

    Ok(result)
}

/// 取得季度財報 ROE、ROA為零的數據
pub async fn fetch_roe_or_roa_equal_to_zero(
    year: Option<i32>,
    quarter: Option<Quarter>,
) -> Result<Vec<FinancialStatement>> {
    let mut sql = String::from(
        r#"
SELECT
    serial,
    security_code,
    year,
    quarter,
    gross_profit,
    operating_profit_margin,
    "pre-tax_income",
    net_income,
    net_asset_value_per_share,
    sales_per_share,
    earnings_per_share,
    profit_before_tax,
    return_on_equity,
    return_on_assets,
    created_time,
    updated_time
FROM financial_statement
WHERE quarter = $1 AND (return_on_equity = 0 OR return_on_assets = 0)
"#,
    );

    if let Some(year) = year {
        sql.push_str(&format!("year = {}", year))
    }

    let q = match quarter {
        None => String::from(""),
        Some(q) => q.to_string(),
    };

    sqlx::query(&sql)
        .bind(q)
        .try_map(|row: PgRow| {
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
        })
        .fetch_all(database::get_connection())
        .await
        .map_err(|why| {
            anyhow!(
                "Failed to fetch_roe_is_zero({:?},{:?}) from database\nsql:{}\n {:?}",
                year,
                quarter,
                &sql,
                why
            )
        })
}

/// 取得沒年報的股票有哪些
pub async fn fetch_without_annual(year: i32) -> Result<Vec<FinancialStatement>> {
    let years: Vec<i32> = (0..10).map(|i| year - i).collect();
    let years_str = years
        .iter()
        .map(|&year| year.to_string())
        .collect::<Vec<String>>()
        .join(",");

    let sql = format!(
        r#"
SELECT DISTINCT
    f1.year,
    f1.security_code
FROM
    financial_statement f1
LEFT JOIN
    financial_statement f2
    ON f1.year = f2.year
    AND f1.security_code = f2.security_code
    AND f2.quarter = ''
WHERE
    f1.year IN ({0}) AND f2.year IS NULL
ORDER BY
    f1.security_code,
    f1.year;
"#,
        years_str
    );

    sqlx::query(&sql)
        .try_map(|row: PgRow| {
            Ok(FinancialStatement {
                updated_time: Default::default(),
                created_time: Default::default(),
                quarter: Default::default(),
                security_code: row.try_get("security_code")?,
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
                serial: Default::default(),
                year: row.try_get("year")?,
            })
        })
        .fetch_all(database::get_connection())
        .await
        .map_err(|why| {
            anyhow!(
                "Failed to fetch_without_annual({}) from database\nsql:{}\n {:?}",
                year,
                &sql,
                why
            )
        })
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<yahoo::profile::Profile> for FinancialStatement {
    fn from(fs: yahoo::profile::Profile) -> Self {
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
    fn from(fs: crawler::share::AnnualProfit) -> Self {
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
mod tests {
    use crate::logging;
    use chrono::{Datelike, NaiveDate};

    use super::*;

    #[tokio::test]
    async fn test_fetch_annual() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_annual".to_string());

        let r = fetch_annual(2022).await;
        if let Ok(result) = r {
            logging::debug_file_async(format!("{:?}", result));
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_annual".to_string());
    }

    #[tokio::test]
    async fn test_fetch_roe_is_zero() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_roe_is_zero".to_string());

        let r = fetch_roe_or_roa_equal_to_zero(Some(2023), Some(Quarter::Q3)).await;
        if let Ok(result) = r {
            dbg!(&result);
            logging::debug_file_async(format!("{:?}", result));
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?}", err));
        }
        logging::debug_file_async("結束 fetch_roe_is_zero".to_string());
    }

    #[tokio::test]
    async fn test_fetch_without_annual() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_without_annual".to_string());

        let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
        let r = fetch_without_annual(current_date.year()).await;
        match r {
            Ok(result) => {
                //dbg!(&result);
                logging::debug_file_async(format!("{:#?}", result));
            }
            Err(err) => {
                logging::debug_file_async(format!("{:#?}", err));
            }
        }
        logging::debug_file_async("結束 fetch_without_annual".to_string());
    }
}
