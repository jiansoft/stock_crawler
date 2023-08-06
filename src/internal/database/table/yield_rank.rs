use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration, NaiveDate};
use sqlx::postgres::PgQueryResult;

use crate::internal::database;

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

        let month_ago = date - Duration::days(30);
        let sql = format!(
            r#"
WITH dividend_max_year AS (
    SELECT
        d.security_code,
        MAX(d."year") AS year
    FROM
        dividend AS d
    WHERE
        "year" >= $1
        AND quarter IN ('')
    GROUP BY
        d.security_code
),
dividend_serial AS (
    SELECT
        d.security_code,
        d."sum",
        d.serial
    FROM
        dividend_max_year AS dmy
        INNER JOIN dividend AS d ON d.security_code = dmy.security_code AND d."year" = dmy.year AND quarter IN ('')
),
daily_quotes_serial AS (
    SELECT
        "SecurityCode",
        MAX("Serial") AS serial
    FROM
        "DailyQuotes"
    WHERE
        "Date" <= $2
        AND "Date" >= $3
    GROUP BY
        "SecurityCode"
)
INSERT INTO yield_rank (date, security_code, daily_quotes_serial, dividend_serial, yield)
SELECT
    '{0}' AS date,
    s.stock_symbol,
    dq."Serial" AS daily_quotes_serial,
    d.serial AS dividend_serial,
    (d."sum" / dq."ClosingPrice") * 100 AS yield
FROM
    stocks AS s
    INNER JOIN dividend_serial AS d ON d.security_code = s.stock_symbol
    INNER JOIN daily_quotes_serial AS dqs ON dqs."SecurityCode" = s.stock_symbol
    INNER JOIN "DailyQuotes" AS dq ON dq."Serial" = dqs.serial
ON CONFLICT (date, security_code) DO UPDATE SET
    yield = EXCLUDED.yield,
    daily_quotes_serial = EXCLUDED.daily_quotes_serial,
    dividend_serial = EXCLUDED.dividend_serial,
    updated_time = now();
"#,
            date
        );

        match sqlx::query(&sql)
            .bind(date.year())
            .bind(date)
            .bind(month_ago)
            .execute(&mut *tx)
            .await
            .context("Failed to YieldRank::upsert from database")
        {
            Ok(pg) => {
                tx.commit().await?;
                Ok(pg)
            }
            Err(why) => {
                tx.rollback().await?;
                Err(anyhow!("{:?}", why))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;
    use chrono::Local;

    use super::*;

    #[tokio::test]
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
