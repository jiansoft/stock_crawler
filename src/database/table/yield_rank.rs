use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, NaiveDate, TimeDelta};
use sqlx::postgres::PgQueryResult;

use crate::database;

#[derive(sqlx::FromRow, Debug, Default)]
pub struct YieldRank {
    pub security_code: String,
    pub daily_quotes_serial: i64,
    pub dividend: f64,
    pub closing_price: f64,
    pub r#yield: f64,
}

impl YieldRank {
    pub async fn upsert(date: NaiveDate) -> Result<PgQueryResult> {
        let mut tx = database::get_tx()
            .await
            .context("Failed to get_tx in yield_rank")?;

        let month_ago = date - TimeDelta::try_days(30).unwrap();
        let sql = r#"
INSERT INTO yield_rank (date, security_code, daily_quotes_serial, dividend_serial, yield)
WITH latest_dividend AS (
    -- 取得每支股票最新年度的股利總和，rn=1 代表最新年度
    SELECT 
        security_code,
        serial,
        "sum",
        ROW_NUMBER() OVER (PARTITION BY security_code ORDER BY "year" DESC) as rn
    FROM dividend
    WHERE "year" >= $1 AND quarter = ''
),
latest_quote AS (
    -- 取得每支股票在指定日期範圍內最新的一筆報價
    SELECT 
        "stock_symbol",
        "Serial",
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
    -- 防止除以零並計算殖利率 (%)
    (d."sum" / NULLIF(dq."ClosingPrice", 0)) * 100 AS yield
FROM stocks AS s
JOIN latest_dividend d ON d.security_code = s.stock_symbol AND d.rn = 1
JOIN latest_quote dq ON dq."stock_symbol" = s.stock_symbol AND dq.rn = 1
ON CONFLICT (date, security_code) DO UPDATE SET
    yield = EXCLUDED.yield,
    daily_quotes_serial = EXCLUDED.daily_quotes_serial,
    dividend_serial = EXCLUDED.dividend_serial,
    updated_time = NOW();
"#;

        let result = sqlx::query(sql)
            .bind(date.year() - 1)
            .bind(date)
            .bind(month_ago)
            .execute(&mut *tx)
            .await;

        match result {
            Ok(pg) => {
                tx.commit().await?;
                Ok(pg)
            }
            Err(why) => {
                tx.rollback().await?;
                Err(anyhow!("Failed to YieldRank::upsert from database: {:?}", why))
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 YieldRank::upsert".to_string());
        let current_date = Local::now().date_naive();
        match YieldRank::upsert(current_date).await {
            Ok(r) => logging::debug_file_async(format!("YieldRank::upsert:{:#?}", r)),
            Err(why) => {
                logging::debug_file_async(format!("Failed to YieldRank::upsert because {:?}", why));
            }
        }

        logging::debug_file_async("結束 YieldRank::upsert".to_string());
    }
}
