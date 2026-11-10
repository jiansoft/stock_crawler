use crate::domain::yield_rank::repository::YieldRankRepository;
use crate::infra::database;
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, TimeDelta};

/// 基於 PostgreSQL 的殖利率排行倉儲實現 (PgYieldRankRepository)。
///
/// 負責透過 SQL 重新計算並更新 `"yield_rank"` 資料表。
pub struct PgYieldRankRepository;

impl PgYieldRankRepository {
    /// 建立新的 PgYieldRankRepository 實例。
    pub fn new() -> Self {
        PgYieldRankRepository
    }
}

impl Default for PgYieldRankRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl YieldRankRepository for PgYieldRankRepository {
    /// 依據指定交易日期，重新計算並重建所有股票的殖利率排行資料。
    async fn rebuild_by_date(&self, date: NaiveDate) -> Result<()> {
        // 取得資料庫交易以保證重建過程的原子性
        let mut tx = database::get_tx()
            .await
            .context("Failed to get_tx in PgYieldRankRepository::rebuild_by_date")?;

        let month_ago = date - TimeDelta::try_days(30).unwrap();
        let sql = r#"
INSERT INTO yield_rank (date, security_code, daily_quotes_serial, dividend_serial, yield)
WITH latest_dividend AS (
    -- 取得每支股票最新年度的股利總和，rn=1 代表最新年度
    SELECT 
        security_code,
        serial,
        "sum",
        stock_dividend,
        "ex-dividend_date2" AS ex_rights_date,
        ROW_NUMBER() OVER (PARTITION BY security_code ORDER BY "year" DESC) as rn
    FROM dividend
    WHERE "year" >= $1 AND quarter = ''
),
latest_quote AS (
    -- 取得每支股票在指定日期範圍內最新的一筆報價
    SELECT
        "stock_symbol",
        "Serial",
        "Date",
        "ClosingPrice",
        ROW_NUMBER() OVER (PARTITION BY "stock_symbol" ORDER BY "Date" DESC, "Serial" DESC) as rn
    FROM "DailyQuotes"
    WHERE "Date" <= $2 AND "Date" >= $3
)
SELECT
    $2 AS date,
    s.stock_symbol,
    dq."Serial" AS daily_quotes_serial,
    d.serial AS dividend_serial,
    -- 防止除以零並計算殖利率 (%)。股利是除權前每一股配的金額，報價已在除權日（含）以後時股數變成
    -- 1 + 股票股利 ÷ 10 倍，每股股利要同比例攤薄，否則配股後的殖利率會被高估。
    COALESCE((d."sum" / NULLIF(dq."ClosingPrice", 0)) * 100, 0)
        / CASE WHEN d.stock_dividend > 0 AND pg_input_is_valid(d.ex_rights_date, 'date') THEN
            CASE WHEN d.ex_rights_date::date <= dq."Date" THEN 1 + d.stock_dividend / 10 ELSE 1 END
          ELSE 1 END AS yield
FROM stocks AS s
JOIN latest_dividend d ON d.security_code = s.stock_symbol AND d.rn = 1
JOIN latest_quote dq ON dq."stock_symbol" = s.stock_symbol AND dq.rn = 1
ON CONFLICT (date, security_code) DO UPDATE SET
    yield = EXCLUDED.yield,
    daily_quotes_serial = EXCLUDED.daily_quotes_serial,
    dividend_serial = EXCLUDED.dividend_serial,
    updated_time = NOW();
"#;

        // 執行批次計算重建 SQL
        let result = sqlx::query(sql)
            .bind(date.year() - 1)
            .bind(date)
            .bind(month_ago)
            .execute(&mut *tx)
            .await;

        match result {
            Ok(_) => {
                // 提交交易
                tx.commit()
                    .await
                    .context("Failed to commit transaction in rebuild_by_date")?;
                Ok(())
            }
            Err(why) => {
                // 回滾交易
                tx.rollback().await.ok();
                Err(anyhow!(
                    "Failed to rebuild yield_rank table in database: {:?}",
                    why
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
