//! 每日風險指標（`daily_risk_metric`）：近 240 筆日線的年化波動度與對加權指數的 beta。
//!
//! [`PgRiskMetricRepository::replace_range`] 收盤後算當天、手動回補時一次算一段日期；條件選股與回測的低波動條件
//! 讀這張表。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::infra::database;

/// 每檔近幾筆日線算一次指標。
pub const RISK_WINDOW: i64 = 240;

/// 樣本至少幾筆才寫入（停牌、新上市不足時沒有資料）。
pub const MIN_RISK_SAMPLES: i64 = 200;

/// 重算 `$1`～`$2`（含）每個交易日的風險指標。
///
/// 日報酬取 `DailyQuotes` 的漲跌幅（以參考價為基準，除權息不會被當成下跌），大盤取加權指數的漲跌點 ÷ 前一日指數；
/// 視窗為每檔截至當天的近 `$3` 筆日線（停牌期間不補），只讀起日往前 `$3` − 1 個指數交易日以後的日線。年化波動度
/// = 樣本標準差 × √`$3`；beta = 日報酬對同日大盤報酬的迴歸斜率（同日沒有指數的列不計入）。樣本少於 `$4` 筆不寫入。
const REPLACE_RISK_METRICS_SQL: &str = r#"
WITH bounds AS (
    SELECT COALESCE(MIN(date), $1::date) AS start
    FROM (
        SELECT date FROM "index"
        WHERE category = 'TAIEX' AND date < $1::date
        ORDER BY date DESC
        LIMIT $3::int - 1
    ) AS prior
), market AS (
    SELECT i.date, (i.change / NULLIF(i.index - i.change, 0))::float8 AS market_return
    FROM "index" i
    CROSS JOIN bounds
    WHERE i.category = 'TAIEX' AND i.date BETWEEN bounds.start AND $2::date
), returns AS (
    SELECT q.stock_symbol, q."Date" AS date, q."ChangeRange"::float8 / 100 AS daily_return, m.market_return
    FROM "DailyQuotes" q
    CROSS JOIN bounds
    JOIN stocks s ON s.stock_symbol = q.stock_symbol AND s.stock_exchange_market_id IN (2, 4)
    LEFT JOIN market m ON m.date = q."Date"
    WHERE q."Date" BETWEEN bounds.start AND $2::date
), rolling AS (
    SELECT stock_symbol, date,
           COUNT(daily_return) OVER w AS samples,
           STDDEV_SAMP(daily_return) OVER w AS deviation,
           REGR_SLOPE(daily_return, market_return) OVER w AS beta
    FROM returns
    WINDOW w AS (PARTITION BY stock_symbol ORDER BY date ROWS BETWEEN $3::int - 1 PRECEDING AND CURRENT ROW)
)
INSERT INTO daily_risk_metric (date, stock_symbol, volatility_percent, beta, samples)
SELECT date, stock_symbol, ROUND((deviation * SQRT($3::float8) * 100)::numeric, 2), ROUND(beta::numeric, 3),
       samples::smallint
FROM rolling
WHERE date BETWEEN $1::date AND $2::date AND samples >= $4 AND deviation IS NOT NULL
"#;

/// `daily_risk_metric` 的 PostgreSQL 倉儲。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgRiskMetricRepository;

impl PgRiskMetricRepository {
    /// 建立倉儲。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 重算 `from`～`to`（含）每個交易日的風險指標，回傳寫入列數。
    ///
    /// 同一個交易內先刪除這段日期再寫入；失敗時交易回滾，保留原本的資料。
    ///
    /// # Errors
    ///
    /// 資料庫查詢或寫入失敗時回傳錯誤。
    pub async fn replace_range(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        let mut tx = database::get_batch_connection().begin().await?;
        sqlx::query("DELETE FROM daily_risk_metric WHERE date BETWEEN $1 AND $2")
            .bind(from)
            .bind(to)
            .execute(&mut *tx)
            .await
            .context("清除 daily_risk_metric 失敗")?;
        let inserted = sqlx::query(REPLACE_RISK_METRICS_SQL)
            .bind(from)
            .bind(to)
            .bind(RISK_WINDOW)
            .bind(MIN_RISK_SAMPLES)
            .execute(&mut *tx)
            .await
            .context("寫入 daily_risk_metric 失敗")?
            .rows_affected();
        tx.commit().await.context("提交 daily_risk_metric 失敗")?;
        Ok(inserted)
    }
}
