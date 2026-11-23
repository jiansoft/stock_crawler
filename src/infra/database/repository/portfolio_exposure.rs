//! 持股穿透曝險的 PostgreSQL 查詢：持有部位與市值、持有 ETF 最新一期的前十大持股。

use anyhow::{Context, Result};
use async_trait::async_trait;
use jiff::civil::Date;

use crate::{
    domain::portfolio::exposure::{EtfConstituent, EtfExposureQuery, ExposurePosition},
    domain::portfolio::view::HoldingView,
    infra::database::{
        self,
        repository::{holding_view, portfolio::PgPortfolioRepository},
    },
};

/// 依代號合併的未賣出部位、名稱與市值（`last_daily_quotes` 收盤價）；`$1` 為範圍內的會員編號。
/// 清單頁快照（`etf_snapshot`）上有的才算 ETF。
const POSITIONS_SQL: &str = r#"
WITH held AS (
    SELECT security_code AS stock_symbol, SUM(share_quantity)::bigint AS shares
    FROM stock_ownership_details
    WHERE NOT is_sold AND member_id = ANY($1)
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
    async fn fetch_exposure_positions(&self, view: &HoldingView) -> Result<Vec<ExposurePosition>> {
        let members = holding_view::member_ids(view).await?;
        let rows: Vec<(String, String, Option<f64>, bool)> = sqlx::query_as(POSITIONS_SQL)
            .bind(&members)
            .fetch_all(database::get_connection())
            .await
            .with_context(|| format!("Failed to fetch exposure positions view={view:?}"))?;
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
        crate::infra::cache::etf_analysis::ETF_ANALYSIS
            .constituents(etf_symbols, async {
                let rows: Vec<(String, Date, String, String, f64)> =
                    sqlx::query_as::<_, (String, jiff_sqlx::Date, String, String, f64)>(
                        CONSTITUENTS_SQL,
                    )
                    .bind(etf_symbols)
                    .fetch_all(database::get_connection())
                    .await
                    .map(|rows| {
                        rows.into_iter()
                            .map(|(c0, c1, c2, c3, c4)| {
                                (
                                    c0,
                                    crate::infra::database::jiff_types::from_wire(c1),
                                    c2,
                                    c3,
                                    c4,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .context("Failed to fetch ETF constituents")?;
                Ok(rows
                    .into_iter()
                    .map(
                        |(etf_symbol, as_of, holding_symbol, holding_name, weight)| {
                            EtfConstituent {
                                etf_symbol,
                                as_of,
                                holding_symbol,
                                holding_name,
                                weight,
                            }
                        },
                    )
                    .collect())
            })
            .await
    }
}
