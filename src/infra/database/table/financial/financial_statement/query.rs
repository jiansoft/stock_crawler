//! `financial_statement` 的查詢：年度財報、ROE／ROA 為零、缺年度財報與累計 EPS。

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::{QueryBuilder, Row, postgres::PgRow};

use super::FinancialStatement;
use crate::{core::declare::Quarter, infra::database};

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
        .try_map(FinancialStatement::row_to_entity)
        .fetch_all(database::get_connection())
        .await?;

    Ok(result)
}

/// 取得季度財報 ROE、ROA為零的數據
pub async fn fetch_roe_or_roa_equal_to_zero(
    year: Option<i32>,
    quarter: Option<Quarter>,
) -> Result<Vec<FinancialStatement>> {
    let mut query_builder = QueryBuilder::new(
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
WHERE quarter = "#,
    );

    let q = match quarter {
        None => String::from(""),
        Some(q) => q.to_string(),
    };
    query_builder.push_bind(q);
    query_builder.push(
        " AND (return_on_equity = 0 OR return_on_assets = 0 OR net_asset_value_per_share = 0)",
    );

    if let Some(year) = year {
        query_builder.push(" AND year = ");
        query_builder.push_bind(year);
    }

    let query = query_builder.build();
    let rows = query
        .try_map(FinancialStatement::row_to_entity)
        .fetch_all(database::get_connection())
        .await
        .map_err(|why| {
            anyhow!(
                "Failed to fetch_roe_or_roa_equal_to_zero({:?},{:?}) from database: {:?}",
                year,
                quarter,
                why
            )
        })?;

    Ok(rows)
}

/// 取得沒年報的股票有哪些
pub async fn fetch_without_annual(year: i32) -> Result<Vec<FinancialStatement>> {
    let years: Vec<i32> = (0..10).map(|i| year - i).collect();

    // 使用 f1.year = ANY($1) 代替 f1.year IN ({})，以利用參數化查詢並移除 AssertSqlSafe
    let sql = r#"
SELECT DISTINCT
    f1.year,
    f1.security_code
FROM
    financial_statement f1
INNER JOIN
    stocks as s on s.stock_symbol = f1.security_code and s."SuspendListing" = false
LEFT JOIN
    financial_statement f2
    ON f1.year = f2.year
    AND f1.security_code = f2.security_code
    AND f2.quarter = ''
WHERE
    f1.year = ANY($1) AND f2.year IS NULL
ORDER BY
    f1.security_code,
    f1.year;
"#;

    sqlx::query(sql)
        .bind(&years)
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
                sql,
                why
            )
        })
}

/// 取得指定年度、指定季別集合的 EPS 累計。
///
/// # Errors
/// 當查詢失敗時回傳錯誤。
pub async fn fetch_cumulative_eps(
    security_code: &str,
    year: i32,
    quarters: Vec<Quarter>,
) -> Result<Decimal> {
    let sql = r#"
        SELECT SUM(earnings_per_share) AS cumulative_eps
        FROM financial_statement
        WHERE year = $1
        AND quarter = ANY($2)
        AND security_code = $3
    "#;

    let quarter_values: Vec<String> = quarters.into_iter().map(|q| q.to_string()).collect();
    // 将 quarters 转换为字符串向量
    //let quarter_strs: Vec<String> = quarters.iter().map(|q| q.to_string()).collect();

    // 執行查詢
    let result: (Option<Decimal>,) = sqlx::query_as(sql)
        .bind(year)
        .bind(&quarter_values)
        .bind(security_code)
        .fetch_one(database::get_connection())
        .await?;

    // 返回查詢結果，如果結果為空則返回 0
    Ok(result.0.unwrap_or_else(|| dec!(0)))
}
