//! 持股穿透曝險的 PostgreSQL 查詢：持有部位與市值、持有 ETF 最新一期的前十大持股。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::portfolio::exposure::{EtfConstituent, EtfExposureQuery, ExposurePosition},
    infra::database::{self, repository::portfolio::PgPortfolioRepository},
};

/// 依代號合併的未賣出部位、名稱與市值（`last_daily_quotes` 收盤價）；`$1 = 0` 時為全部會員。
/// 清單頁快照（`etf_snapshot`）上有的才算 ETF。
const POSITIONS_SQL: &str = r#"
WITH held AS (
    SELECT security_code AS stock_symbol, SUM(share_quantity)::bigint AS shares
    FROM stock_ownership_details
    WHERE NOT is_sold AND ($1::bigint = 0 OR member_id = $1)
    GROUP BY security_code
)
SELECT held.stock_symbol::text AS stock_symbol,
       COALESCE(e.name, s."Name", held.stock_symbol::text) AS name,
       CASE WHEN q.closing_price > 0 THEN (held.shares * q.closing_price)::float8 END AS market_value,
       e.stock_symbol IS NOT NULL AS is_etf
FROM held
LEFT JOIN stocks AS s ON s.stock_symbol = held.stock_symbol
LEFT JOIN last_daily_quotes AS q ON q.stock_symbol = held.stock_symbol
LEFT JOIN etf_snapshot AS e ON e.stock_symbol = held.stock_symbol
WHERE held.shares > 0
ORDER BY held.stock_symbol
"#;

/// 指定 ETF 的前十大持股（`etf_holding` 只留最新一期），依排名。
const CONSTITUENTS_SQL: &str = r#"
SELECT stock_symbol::text AS etf_symbol, as_of, holding_symbol::text AS holding_symbol, holding_name,
       weight::float8 AS weight
FROM etf_holding
WHERE stock_symbol = ANY($1)
ORDER BY stock_symbol, rank
"#;

#[async_trait]
impl EtfExposureQuery for PgPortfolioRepository {
    async fn fetch_exposure_positions(&self, member_id: i64) -> Result<Vec<ExposurePosition>> {
        let rows: Vec<(String, String, Option<f64>, bool)> = sqlx::query_as(POSITIONS_SQL)
            .bind(member_id)
            .fetch_all(database::get_connection())
            .await
            .with_context(|| format!("Failed to fetch exposure positions member_id={member_id}"))?;
        Ok(rows
            .into_iter()
            .map(
                |(stock_symbol, name, market_value, is_etf)| ExposurePosition {
                    stock_symbol,
                    name: name.replace("&amp;", "&"),
                    market_value,
                    is_etf,
                },
            )
            .collect())
    }

    async fn fetch_etf_constituents(&self, etf_symbols: &[String]) -> Result<Vec<EtfConstituent>> {
        let rows: Vec<(String, NaiveDate, String, String, f64)> = sqlx::query_as(CONSTITUENTS_SQL)
            .bind(etf_symbols)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch ETF constituents")?;
        Ok(rows
            .into_iter()
            .map(
                |(etf_symbol, as_of, holding_symbol, holding_name, weight)| EtfConstituent {
                    etf_symbol,
                    as_of,
                    holding_symbol,
                    holding_name,
                    weight,
                },
            )
            .collect())
    }
}
