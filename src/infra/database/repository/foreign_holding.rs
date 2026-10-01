//! 外資持股歷史與趨勢的 PostgreSQL 倉儲實作。

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::Row;

use crate::domain::foreign_holding::entity::{ForeignHolding, HoldingForeignHoldingAlert};
use crate::domain::foreign_holding::repository::ForeignHoldingRepository;
use crate::infra::database;

/// 對應資料表 `qfii_history`（每日快照）與 `qfii_trend`（最新趨勢）。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgForeignHoldingRepository;

impl PgForeignHoldingRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }

    /// 以 `as_of` 為基準日重算外資持股趨勢，回傳寫入筆數。
    ///
    /// 只有在基準日當天仍有快照的股票才會寫入；基準日之前的舊趨勢列（已下市、當天無資料）
    /// 一併刪除。以指定日期為準而非全表最新日，回補歷史時才能重算任一天，
    /// 整合測試用固定的歷史日期也不會動到其他日期的趨勢列。
    pub async fn rebuild_trends_as_of(&self, as_of: NaiveDate) -> Result<u64> {
        // 窗口取基準日往前 120 個日曆日（約 80 個交易日），足以涵蓋 20 日變化；
        // 連續天數最多只算到這個窗口為止。
        let upsert = r#"
WITH recent AS (
    SELECT h.stock_symbol, h."date", h.shares_held, h.share_holding_percentage,
        ROW_NUMBER() OVER (PARTITION BY h.stock_symbol ORDER BY h."date" DESC) AS rn,
        SIGN(h.shares_held - LAG(h.shares_held) OVER (PARTITION BY h.stock_symbol ORDER BY h."date")) AS dir
    FROM qfii_history h
    WHERE h."date" <= $1 AND h."date" > $1 - 120
),
per_stock AS (
    SELECT stock_symbol,
        MAX("date") FILTER (WHERE rn = 1) AS last_date,
        MAX(share_holding_percentage) FILTER (WHERE rn = 1) AS pct,
        MAX(share_holding_percentage) FILTER (WHERE rn = 6) AS pct_5,
        MAX(share_holding_percentage) FILTER (WHERE rn = 21) AS pct_20,
        MAX(dir) FILTER (WHERE rn = 1) AS last_dir
    FROM recent
    GROUP BY stock_symbol
),
streaks AS (
    -- 從最新一筆往回數，方向與最新一筆相同的連續筆數；第一個不同（或無法比較）的位置減 1 即是天數
    SELECT r.stock_symbol,
        CASE WHEN COALESCE(p.last_dir, 0) = 0 THEN 0
             ELSE (p.last_dir * (MIN(r.rn) FILTER (WHERE r.dir IS DISTINCT FROM p.last_dir) - 1))::int
        END AS streak
    FROM recent r
    JOIN per_stock p ON p.stock_symbol = r.stock_symbol
    GROUP BY r.stock_symbol, p.last_dir
)
INSERT INTO qfii_trend (stock_symbol, "date", share_holding_percentage, change_5d, change_20d, streak, updated_time)
SELECT p.stock_symbol, p.last_date, p.pct, p.pct - p.pct_5, p.pct - p.pct_20, s.streak, now()
FROM per_stock p
JOIN streaks s ON s.stock_symbol = p.stock_symbol
WHERE p.last_date = $1
ON CONFLICT (stock_symbol) DO UPDATE SET
    "date" = EXCLUDED."date",
    share_holding_percentage = EXCLUDED.share_holding_percentage,
    change_5d = EXCLUDED.change_5d,
    change_20d = EXCLUDED.change_20d,
    streak = EXCLUDED.streak,
    updated_time = now()
"#;
        let delete_stale = r#"DELETE FROM qfii_trend WHERE "date" < $1"#;

        let mut tx = database::get_tx().await?;
        let written = sqlx::query(upsert)
            .bind(as_of)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to rebuild qfii_trend as of {as_of}"))?
            .rows_affected();
        sqlx::query(delete_stale)
            .bind(as_of)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to delete stale qfii_trend before {as_of}"))?;
        tx.commit()
            .await
            .context("Failed to commit qfii_trend rebuild")?;

        Ok(written)
    }
}

#[async_trait]
impl ForeignHoldingRepository for PgForeignHoldingRepository {
    async fn save_daily(&self, holdings: &[ForeignHolding]) -> Result<u64> {
        if holdings.is_empty() {
            return Ok(0);
        }

        // 同一批次內 (代號, 日期) 重複時 ON CONFLICT DO UPDATE 會直接報錯，先以最後一筆為準去重。
        let mut unique: HashMap<(&str, NaiveDate), &ForeignHolding> = HashMap::new();
        for holding in holdings {
            unique.insert((holding.stock_symbol.as_str(), holding.date), holding);
        }

        let mut symbols = Vec::with_capacity(unique.len());
        let mut dates = Vec::with_capacity(unique.len());
        let mut issued = Vec::with_capacity(unique.len());
        let mut held = Vec::with_capacity(unique.len());
        let mut percentages: Vec<Decimal> = Vec::with_capacity(unique.len());
        for holding in unique.values() {
            symbols.push(holding.stock_symbol.clone());
            dates.push(holding.date);
            issued.push(holding.issued_share);
            held.push(holding.shares_held);
            percentages.push(holding.share_holding_percentage);
        }

        let sql = r#"
INSERT INTO qfii_history (stock_symbol, "date", issued_share, shares_held, share_holding_percentage)
SELECT * FROM UNNEST($1::varchar[], $2::date[], $3::bigint[], $4::bigint[], $5::numeric[])
ON CONFLICT (stock_symbol, "date") DO UPDATE SET
    issued_share = EXCLUDED.issued_share,
    shares_held = EXCLUDED.shares_held,
    share_holding_percentage = EXCLUDED.share_holding_percentage,
    updated_time = now()
WHERE (qfii_history.issued_share, qfii_history.shares_held, qfii_history.share_holding_percentage)
    IS DISTINCT FROM (EXCLUDED.issued_share, EXCLUDED.shares_held, EXCLUDED.share_holding_percentage)
"#;

        let result = sqlx::query(sql)
            .bind(&symbols)
            .bind(&dates)
            .bind(&issued)
            .bind(&held)
            .bind(&percentages)
            .execute(database::get_connection())
            .await
            .context("Failed to save qfii_history")?;

        Ok(result.rows_affected())
    }

    async fn rebuild_trends(&self) -> Result<u64> {
        let latest: Option<NaiveDate> =
            sqlx::query_scalar(r#"SELECT MAX("date") FROM qfii_history"#)
                .fetch_one(database::get_connection())
                .await
                .context("Failed to fetch latest qfii_history date")?;

        match latest {
            Some(as_of) => self.rebuild_trends_as_of(as_of).await,
            None => Ok(0),
        }
    }

    async fn fetch_holding_alerts(&self) -> Result<Vec<HoldingForeignHoldingAlert>> {
        let sql = r#"
SELECT t.stock_symbol, s."Name" AS stock_name, t."date", t.share_holding_percentage,
    t.change_5d, t.change_20d, t.streak
FROM qfii_trend t
JOIN stocks s ON s.stock_symbol = t.stock_symbol
WHERE t.stock_symbol IN (
    SELECT security_code FROM stock_ownership_details WHERE is_sold = false
)
ORDER BY t.change_20d DESC NULLS LAST, t.stock_symbol
"#;

        let rows = sqlx::query(sql)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch holding foreign holding alerts")?;

        rows.iter()
            .map(|row| {
                Ok(HoldingForeignHoldingAlert {
                    stock_symbol: row.try_get("stock_symbol")?,
                    stock_name: row.try_get("stock_name")?,
                    date: row.try_get("date")?,
                    share_holding_percentage: row.try_get("share_holding_percentage")?,
                    change_5d: row.try_get("change_5d")?,
                    change_20d: row.try_get("change_20d")?,
                    streak: row.try_get("streak")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    const FAKE_SYMBOL: &str = "79979QF";

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
    }

    async fn cleanup() {
        let pool = database::get_connection();
        let _ = sqlx::query("DELETE FROM qfii_history WHERE stock_symbol = $1")
            .bind(FAKE_SYMBOL)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM qfii_trend WHERE stock_symbol = $1")
            .bind(FAKE_SYMBOL)
            .execute(pool)
            .await;
    }

    fn holding(day: NaiveDate, held: i64, pct: Decimal) -> ForeignHolding {
        ForeignHolding {
            stock_symbol: FAKE_SYMBOL.to_string(),
            date: day,
            issued_share: 1_000_000,
            shares_held: held,
            share_holding_percentage: pct,
        }
    }

    /// 21 個交易日的假資料：前 18 天持平、最後 3 天連續增持。
    /// 驗證 5 日／20 日變化與連續天數，以及重送同一天不重複計數。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_save_daily_and_rebuild_trend() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_save_daily_and_rebuild_trend：無資料庫連接");
            return;
        }
        cleanup().await;
        let repo = PgForeignHoldingRepository::new();

        let start = date(1990, 1, 1);
        let mut rows = Vec::new();
        for i in 0..21_i64 {
            let day = start + chrono::Days::new(i as u64);
            let (held, pct) = match i {
                0..=17 => (100_000, dec!(10)),
                18 => (110_000, dec!(11)),
                19 => (120_000, dec!(12)),
                _ => (130_000, dec!(13)),
            };
            rows.push(holding(day, held, pct));
        }
        assert_eq!(repo.save_daily(&rows).await.expect("save"), 21);
        // 內容相同的重送不算變更
        assert_eq!(repo.save_daily(&rows).await.expect("save again"), 0);

        let as_of = start + chrono::Days::new(20);
        assert!(repo.rebuild_trends_as_of(as_of).await.expect("rebuild") >= 1);

        let row = sqlx::query(
            "SELECT change_5d, change_20d, streak, share_holding_percentage FROM qfii_trend WHERE stock_symbol = $1",
        )
        .bind(FAKE_SYMBOL)
        .fetch_one(database::get_connection())
        .await
        .expect("trend row");
        let change_5d: Option<Decimal> = row.get("change_5d");
        let change_20d: Option<Decimal> = row.get("change_20d");
        let streak: i32 = row.get("streak");
        assert_eq!(change_5d, Some(dec!(3)));
        assert_eq!(change_20d, Some(dec!(3)));
        assert_eq!(streak, 3);

        cleanup().await;
    }
}
