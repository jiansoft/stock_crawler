//! 每日股價淨值比歷史分位（`daily_pb_percentile`）：當天官方股價淨值比在自身近 5 年的百分位。
//!
//! [`PgPbPercentileRepository::replace_range`] 收盤後算當天、手動回補時一次算一段日期；條件選股與回測的
//! `max_pb_percentile` 讀這張表。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::infra::database;

/// 往回看幾個日曆年。
pub const PB_PERCENTILE_YEARS: i32 = 5;

/// 樣本至少幾筆才寫入（約 2 年；新上市、長期停牌或沒有股價淨值比時沒有資料）。
pub const MIN_PB_SAMPLES: i64 = 480;

/// 重算 `$1`～`$2`（含）每個交易日的股價淨值比分位。
///
/// 樣本為上市櫃每檔截至當天、近 `$3` 個日曆年（不含 `$3` 年前的同一天）的官方股價淨值比，只算大於 0 的日線；
/// 百分位 = 比當天低的天數 ÷ (樣本數 − 1) × 100（0 為區間最低、100 為最高）。樣本少於 `$4` 筆不寫入。
const REPLACE_PB_PERCENTILES_SQL: &str = r#"
WITH hist AS MATERIALIZED (
    SELECT q.stock_symbol, q."Date" AS date, q."price-to-book_ratio"::float8 AS pb
    FROM "DailyQuotes" q
    JOIN stocks s ON s.stock_symbol = q.stock_symbol AND s.stock_exchange_market_id IN (2, 4)
    WHERE q."Date" > $1::date - make_interval(years => $3::int)
      AND q."Date" <= $2::date
      AND q."price-to-book_ratio" > 0
)
INSERT INTO daily_pb_percentile (date, stock_symbol, pb, pb_percentile, samples)
SELECT t.date, t.stock_symbol, ROUND(t.pb::numeric, 2),
       ROUND((COUNT(*) FILTER (WHERE h.pb < t.pb) * 100.0 / (COUNT(*) - 1))::numeric, 2),
       COUNT(*)::smallint
FROM hist t
JOIN hist h
  ON h.stock_symbol = t.stock_symbol
 AND h.date > t.date - make_interval(years => $3::int)
 AND h.date <= t.date
WHERE t.date BETWEEN $1::date AND $2::date
GROUP BY t.date, t.stock_symbol, t.pb
HAVING COUNT(*) >= $4
"#;

/// `daily_pb_percentile` 的 PostgreSQL 倉儲。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgPbPercentileRepository;

impl PgPbPercentileRepository {
    /// 建立倉儲。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 重算 `from`～`to`（含）每個交易日的股價淨值比分位，回傳寫入列數。
    ///
    /// 同一個交易內先刪除這段日期再寫入；失敗時交易回滾，保留原本的資料。
    ///
    /// # Errors
    ///
    /// 資料庫查詢或寫入失敗時回傳錯誤。
    pub async fn replace_range(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        let mut tx = database::get_batch_connection().begin().await?;
        sqlx::query("DELETE FROM daily_pb_percentile WHERE date BETWEEN $1 AND $2")
            .bind(from)
            .bind(to)
            .execute(&mut *tx)
            .await
            .context("清除 daily_pb_percentile 失敗")?;
        let inserted = sqlx::query(REPLACE_PB_PERCENTILES_SQL)
            .bind(from)
            .bind(to)
            .bind(PB_PERCENTILE_YEARS)
            .bind(MIN_PB_SAMPLES)
            .execute(&mut *tx)
            .await
            .context("寫入 daily_pb_percentile 失敗")?
            .rows_affected();
        tx.commit().await.context("提交 daily_pb_percentile 失敗")?;
        Ok(inserted)
    }
}
