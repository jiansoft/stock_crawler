//! 證券主檔的財報指標：近四季 EPS／ROE 回寫，以及每股淨值為零、缺季報的清單。

use anyhow::{Context, Result};

use crate::domain::registry::entity::Stock;
use crate::infra::database;
use crate::infra::database::table::stock::StockDbRow;

/// 更新個股最新一季與近四季的 EPS、ROE 等財務指標。
pub(super) async fn update_eps_and_roe() -> Result<()> {
    let sql = r#"
WITH fs_data AS (
    SELECT
        row_number() OVER (
            PARTITION BY security_code
            ORDER BY year DESC, quarter DESC
        ) AS row_number,
        serial
    FROM
        financial_statement
    WHERE
        year IN ($1, $2)
        AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
),
relevant_fs_rows AS (
    SELECT
        fs_data.row_number,
        fs.security_code,
        fs.earnings_per_share,
        fs.net_asset_value_per_share,
        fs.return_on_equity
    FROM
        financial_statement fs
    JOIN
        fs_data ON fs_data.serial = fs.serial
    ORDER BY year DESC, quarter DESC
),
aggregated_eps AS (
    SELECT
        security_code,
        SUM(earnings_per_share) AS last_four_eps
    FROM
        relevant_fs_rows
    WHERE
        row_number <= 4
    GROUP BY
        security_code
)
UPDATE
    stocks
SET
    last_four_eps = agg.last_four_eps,
    last_one_eps = current_row.earnings_per_share,
    net_asset_value_per_share = current_row.net_asset_value_per_share,
    return_on_equity = current_row.return_on_equity
FROM
    relevant_fs_rows AS current_row
JOIN
    aggregated_eps AS agg ON current_row.security_code = agg.security_code
WHERE
    current_row.security_code = stocks.stock_symbol;
"#;
    use chrono::{Datelike, Local, TimeDelta};
    let now = Local::now();
    let one_year_ago = now - TimeDelta::try_days(365).unwrap();

    sqlx::query(sql)
        .bind(now.year())
        .bind(one_year_ago.year())
        .execute(database::get_connection())
        .await
        .context("Failed to update_eps_and_roe in PgStockRepository")?;
    Ok(())
}

/// 取得所有每股淨值為零的非下市證券主檔。
pub(super) async fn fetch_net_asset_value_per_share_is_zero() -> Result<Vec<Stock>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    s.return_on_equity,
    s.stock_exchange_market_id,
    s.stock_industry_id,
    s.weight,
    s.issued_share,
    s.qfii_shares_held,
    s.qfii_share_holding_percentage
FROM stocks AS s
WHERE s.stock_exchange_market_id in (2, 4)
    AND s."SuspendListing" = false
    AND s.stock_industry_id != $1
    AND s.net_asset_value_per_share = 0
"#;
    use crate::core::declare::Industry;
    let rows = sqlx::query_as::<_, StockDbRow>(sql)
        .bind(Industry::ExchangeTradedFund.serial())
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_net_asset_value_per_share_is_zero in PgStockRepository")?;

    let list = rows
        .into_iter()
        .map(|row| {
            Stock::reconstitute(
                row.stock_symbol,
                row.name,
                row.suspend_listing,
                row.net_asset_value_per_share,
                row.weight,
                row.return_on_equity,
                row.create_time,
                row.stock_exchange_market_id,
                row.stock_industry_id,
                row.issued_share,
                row.qfii_shares_held,
                row.qfii_share_holding_percentage,
            )
        })
        .collect();
    Ok(list)
}

/// 取得指定年度與季別中，缺漏財務報表的證券代號清單。
pub(super) async fn fetch_stocks_without_financial_statement(
    year: i32,
    quarter: &str,
) -> Result<Vec<String>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    s.return_on_equity,
    s.stock_exchange_market_id,
    s.stock_industry_id,
    s.weight,
    s.issued_share,
    s.qfii_shares_held,
    s.qfii_share_holding_percentage
FROM stocks AS s
WHERE s.stock_exchange_market_id in(2, 4)
    AND s."SuspendListing" = false
    AND s.stock_industry_id != $3
    AND NOT EXISTS (
        SELECT 1
        FROM financial_statement f
        WHERE f.security_code = s.stock_symbol
        AND f.year = $1
        AND f.quarter = $2
        AND earnings_per_share > 0
    )
"#;
    use crate::core::declare::Industry;
    let rows = sqlx::query_as::<_, StockDbRow>(sql)
        .bind(year)
        .bind(quarter)
        .bind(Industry::ExchangeTradedFund.serial())
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_stocks_without_financial_statement in PgStockRepository")?;

    let list = rows.into_iter().map(|row| row.stock_symbol).collect();
    Ok(list)
}
