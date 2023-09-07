use anyhow::{Context, Result};
use chrono::NaiveDate;
use sqlx::postgres::PgQueryResult;

use crate::internal::database;

#[derive(sqlx::FromRow, Debug, Default)]
pub struct Estimate {
    pub date: NaiveDate,
    // 使用 chrono 庫來處理日期和時間
    pub last_daily_quote_date: String,
    pub security_code: String,
    pub name: String,
    pub closing_price: f64,
    pub percentage: f64,
    pub cheap: f64,
    pub fair: f64,
    pub expensive: f64,
    pub price_cheap: f64,
    pub price_fair: f64,
    pub price_expensive: f64,
    pub dividend_cheap: f64,
    pub dividend_fair: f64,
    pub dividend_expensive: f64,
    pub year_count: i32,
    pub index: i32,
}

impl Estimate {
    pub fn new(security_code: String, date: NaiveDate) -> Self {
        Estimate {
            date,
            last_daily_quote_date: "".to_string(),
            security_code,
            name: "".to_string(),
            closing_price: 0.0,
            percentage: 0.0,
            cheap: 0.0,
            fair: 0.0,
            expensive: 0.0,
            price_cheap: 0.0,
            price_fair: 0.0,
            price_expensive: 0.0,
            dividend_cheap: 0.0,
            dividend_fair: 0.0,
            dividend_expensive: 0.0,
            year_count: 0,
            index: 0,
        }
    }

    pub async fn upsert(&self, years: String) -> Result<PgQueryResult> {
        let sql = format!(
            r#"
WITH params AS (
    SELECT
        array[{0}]::int[] AS year_filter,
        '{1}' AS security_code_filter,
        '{2}'::date AS date_filter
),
price as (
    SELECT
        "SecurityCode" AS security_code,
        COUNT(DISTINCT "year") AS year_count,
        PERCENTILE_CONT(0.2) WITHIN GROUP (ORDER BY "LowestPrice") AS price_cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "ClosingPrice") AS price_fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY "HighestPrice") AS price_expensive
    FROM params AS p
    INNER JOIN "DailyQuotes" AS dq ON p.security_code_filter = dq."SecurityCode"
                                   AND "Date" <= p.date_filter
                                   AND "year" = ANY(p.year_filter)
                                   AND "ClosingPrice" > 0
    GROUP BY "SecurityCode"
),
dividend as (
	SELECT
        d.security_code,
        AVG(d."sum") * 15 AS dividend_cheap,
        AVG(d."sum") * 20 AS dividend_fair,
        AVG(d."sum") * 25 AS dividend_expensive
    FROM params AS p
    JOIN dividend d ON p.security_code_filter = d.security_code AND d."year" = ANY(p.year_filter)
    GROUP BY d.security_code
),
dividend_payout_ratio as (
	SELECT
        d.security_code,
        COALESCE(PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY d.payout_ratio), 70) AS payout_ratio
    FROM params AS p
    LEFT JOIN public.dividend d ON p.security_code_filter = d.security_code
                                AND d."year" = ANY(p.year_filter)
                                AND d.payout_ratio > 0
                                AND d.payout_ratio <= 100
    GROUP BY d.security_code
),
eps as (
    SELECT
        s.stock_symbol,
        s.last_four_eps * (dpr.payout_ratio / 100) * 15 as eps_cheap,
        s.last_four_eps * (dpr.payout_ratio / 100) * 20 as eps_fair,
        s.last_four_eps * (dpr.payout_ratio / 100) * 25 as eps_expensive
    FROM params AS p
    JOIN stocks s ON p.security_code_filter = s.stock_symbol
    JOIN dividend_payout_ratio dpr ON p.security_code_filter = dpr.security_code

)
INSERT INTO estimate (
    security_code, "date", percentage, closing_price, cheap, fair, expensive, price_cheap,
    price_fair, price_expensive, dividend_cheap, dividend_fair, dividend_expensive, year_count,
    eps_cheap, eps_fair, eps_expensive, update_time
)
SELECT
    dq."SecurityCode",
    dq."Date",
    (((dq."ClosingPrice" / ((price_cheap + dividend_cheap + eps.eps_cheap) / 3))) * 100) as percentage,
    dq."ClosingPrice",
    (price_cheap + dividend_cheap + eps.eps_cheap) / 3             as cheap,
    (price_fair + dividend_fair + eps.eps_fair) / 3                as fair,
    (price_expensive + dividend_expensive + eps.eps_expensive) / 3 as expensive,
    price_cheap,
    price_fair,
    price_expensive,
    dividend_cheap,
    dividend_fair,
    dividend_expensive,
    year_count,
    eps_cheap,
    eps_fair,
    eps_expensive,
    NOW()
FROM params AS p
INNER JOIN stocks AS s ON p.security_code_filter = s.stock_symbol
INNER JOIN "DailyQuotes" AS dq ON p.security_code_filter = dq."SecurityCode" and dq."Date" = p.date_filter
INNER JOIN price ON p.security_code_filter = price.security_code
INNER JOIN dividend ON p.security_code_filter = dividend.security_code
INNER JOIN eps ON p.security_code_filter = eps.stock_symbol
ON CONFLICT (date, security_code) DO UPDATE SET
    percentage = EXCLUDED.percentage,
    closing_price = EXCLUDED.closing_price,
    cheap = EXCLUDED.cheap,
    fair = EXCLUDED.fair,
    expensive = EXCLUDED.expensive,
    price_cheap = EXCLUDED.price_cheap,
    price_fair = EXCLUDED.price_fair,
    price_expensive = EXCLUDED.price_expensive,
    dividend_cheap = EXCLUDED.dividend_cheap,
    dividend_fair = EXCLUDED.dividend_fair,
    dividend_expensive = EXCLUDED.dividend_expensive,
    eps_cheap = EXCLUDED.eps_cheap,
    eps_fair = EXCLUDED.eps_fair,
    eps_expensive = EXCLUDED.eps_expensive,
    year_count = EXCLUDED.year_count,
    update_time = NOW();
"#,
            years, &self.security_code, self.date
        );

        sqlx::query(&sql)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to upsert estimate({:#?}) from database\nsql:{}",
                self, sql
            ))
    }
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use crate::internal::logging;
    use chrono::Datelike;

    use super::*;

    #[tokio::test]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 Estimate::upsert".to_string());

        let current_date = NaiveDate::parse_from_str("2023-08-18", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        let estimate = Estimate::new("2330".to_string(), current_date);

        match estimate.upsert(years_str).await {
            Ok(r) => logging::debug_file_async(format!("Estimate::upsert:{:#?}", r)),
            Err(why) => {
                logging::debug_file_async(format!("Failed to Estimate::upsert because {:?}", why));
            }
        }

        logging::debug_file_async("結束 Estimate::upsert".to_string());
    }
}
