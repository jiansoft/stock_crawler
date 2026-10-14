//! 相對強度歷史（[`RelativeStrengthQuery`]）的 PostgreSQL 實作。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::quote::relative_strength::{RelativeStrengthDay, RelativeStrengthQuery},
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// `$1`（含）之前最近 `$2` 個交易日，每天以上市櫃全體計分後，取股票 `$3` 的加權報酬與名次。
///
/// 交易日曆取 `$2 + 240` 個交易日（回看 800 天足以涵蓋 120 + 240 個交易日），資料日 T 的
/// 近 N 日報酬＝往前第 N 個交易日（基準日）的 N 日報酬；`weaker` 為加權報酬比它低的家數，
/// 由 domain 換算成 1–99 分。母體條件與排行相同：上市櫃、未暫停交易、三個天期的報酬都有。
/// 報酬還沒回填的交易日沒有任何列，自然略過。
const HISTORY_SQL: &str = r#"
WITH td AS (
    SELECT date, (ROW_NUMBER() OVER (ORDER BY date DESC) - 1)::int AS idx
    FROM (
        SELECT DISTINCT "Date" AS date
        FROM "DailyQuotes"
        WHERE "Date" <= $1 AND "Date" > $1 - 800
        ORDER BY 1 DESC
        LIMIT $2::int + 240
    ) recent
),
ends AS (
    SELECT e.date AS end_date, b60.date AS d60, b120.date AS d120, b240.date AS d240
    FROM td e
    JOIN td b60 ON b60.idx = e.idx + 60
    JOIN td b120 ON b120.idx = e.idx + 120
    JOIN td b240 ON b240.idx = e.idx + 240
    WHERE e.idx < $2::int
),
scored AS (
    SELECT e.end_date, r60.stock_symbol,
           0.4 * r60.return_60d::float8 + 0.3 * r120.return_120d::float8
               + 0.3 * r240.return_240d::float8 AS weighted
    FROM ends e
    JOIN daily_forward_return r60 ON r60.date = e.d60
    JOIN daily_forward_return r120 ON r120.date = e.d120 AND r120.stock_symbol = r60.stock_symbol
    JOIN daily_forward_return r240 ON r240.date = e.d240 AND r240.stock_symbol = r60.stock_symbol
    JOIN stocks s ON s.stock_symbol = r60.stock_symbol
    WHERE r60.return_60d IS NOT NULL
      AND r120.return_120d IS NOT NULL
      AND r240.return_240d IS NOT NULL
      AND s.stock_exchange_market_id IN (2, 4)
      AND NOT s."SuspendListing"
),
ranked AS (
    SELECT end_date, stock_symbol, weighted,
           RANK() OVER (PARTITION BY end_date ORDER BY weighted) - 1 AS weaker,
           RANK() OVER (PARTITION BY end_date ORDER BY weighted DESC) AS rank,
           COUNT(*) OVER (PARTITION BY end_date) AS universe
    FROM scored
)
SELECT stock_symbol, end_date AS date, weighted, weaker, rank, universe
FROM ranked
WHERE stock_symbol = ANY($3)
ORDER BY stock_symbol, end_date DESC
"#;

/// [`HISTORY_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct HistoryRow {
    stock_symbol: String,
    date: NaiveDate,
    weighted: f64,
    weaker: i64,
    rank: i64,
    universe: i64,
}

#[async_trait]
impl RelativeStrengthQuery for PgQuoteRepository {
    async fn fetch_relative_strength(
        &self,
        until: NaiveDate,
        days: u16,
        stock_symbols: &[String],
    ) -> Result<Vec<RelativeStrengthDay>> {
        let rows: Vec<HistoryRow> = sqlx::query_as(HISTORY_SQL)
            .bind(until)
            .bind(i32::from(days))
            .bind(stock_symbols)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| RelativeStrengthDay {
                stock_symbol: row.stock_symbol,
                date: row.date,
                weighted: row.weighted,
                weaker: row.weaker,
                rank: row.rank,
                universe: row.universe,
            })
            .collect())
    }
}
