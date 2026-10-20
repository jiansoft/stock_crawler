//! 市場估值水位（`daily_market_valuation`）的 PostgreSQL 實作（[`MarketValuationRepository`]）。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::market_index::valuation::{MarketValuationDay, MarketValuationRepository},
    infra::database,
};

/// 由日線重算 `$1`～`$2` 每個交易日的中位數：上市、上櫃各一列，`GROUPING SETS` 另出一列合計
/// （市場編號 0）。只計大於 0 的官方值；同一天同市場覆蓋。
const REFRESH_SQL: &str = r#"
INSERT INTO daily_market_valuation
    (date, stock_exchange_market_id, median_pe, median_pb, pe_samples, pb_samples, updated_at)
SELECT q."Date",
       CASE WHEN GROUPING(s.stock_exchange_market_id) = 1 THEN 0 ELSE s.stock_exchange_market_id END,
       PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY q."PriceEarningRatio")
           FILTER (WHERE q."PriceEarningRatio" > 0),
       PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY q."price-to-book_ratio")
           FILTER (WHERE q."price-to-book_ratio" > 0),
       COUNT(*) FILTER (WHERE q."PriceEarningRatio" > 0),
       COUNT(*) FILTER (WHERE q."price-to-book_ratio" > 0),
       NOW()
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $1 AND $2
  AND s.stock_exchange_market_id IN (2, 4)
  AND (q."PriceEarningRatio" > 0 OR q."price-to-book_ratio" > 0)
GROUP BY GROUPING SETS ((q."Date", s.stock_exchange_market_id), (q."Date"))
ON CONFLICT (date, stock_exchange_market_id) DO UPDATE
SET median_pe = EXCLUDED.median_pe,
    median_pb = EXCLUDED.median_pb,
    pe_samples = EXCLUDED.pe_samples,
    pb_samples = EXCLUDED.pb_samples,
    updated_at = EXCLUDED.updated_at
"#;

/// `$1` 市場在 `$2`～`$3` 的每日中位數，由舊到新。
const FETCH_SQL: &str = r#"
SELECT date, median_pe::float8 AS median_pe, median_pb::float8 AS median_pb,
       pe_samples::int8 AS pe_samples, pb_samples::int8 AS pb_samples
FROM daily_market_valuation
WHERE stock_exchange_market_id = $1 AND date BETWEEN $2 AND $3
ORDER BY date
"#;

/// [`FETCH_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct ValuationRow {
    date: NaiveDate,
    median_pe: Option<f64>,
    median_pb: Option<f64>,
    pe_samples: i64,
    pb_samples: i64,
}

/// `daily_market_valuation` 的倉儲。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgMarketValuationRepository;

impl PgMarketValuationRepository {
    /// 建立倉儲。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MarketValuationRepository for PgMarketValuationRepository {
    async fn refresh_market_valuation(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        // 一年全市場約 50 萬筆日線；用批次連線池（慢查詢門檻較高）。
        let result = sqlx::query(REFRESH_SQL)
            .bind(from)
            .bind(to)
            .execute(database::get_batch_connection())
            .await
            .with_context(|| format!("重算 {from}～{to} 的市場估值水位失敗"))?;
        Ok(result.rows_affected())
    }

    async fn fetch_market_valuation(
        &self,
        market_id: i32,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<MarketValuationDay>> {
        let rows: Vec<ValuationRow> = sqlx::query_as(FETCH_SQL)
            .bind(market_id)
            .bind(from)
            .bind(to)
            .fetch_all(database::get_connection())
            .await
            .context("讀取市場估值水位失敗")?;
        Ok(rows
            .into_iter()
            .map(|row| MarketValuationDay {
                date: row.date,
                median_pe: row.median_pe,
                median_pb: row.median_pb,
                pe_samples: row.pe_samples,
                pb_samples: row.pb_samples,
            })
            .collect())
    }
}
