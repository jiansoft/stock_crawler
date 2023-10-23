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
    pub eps_cheap: f64,
    pub eps_fair: f64,
    pub eps_expensive: f64,
    pub pbr_cheap: f64,
    pub pbr_fair: f64,
    pub pbr_expensive: f64,
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
            eps_cheap: 0.0,
            eps_fair: 0.0,
            eps_expensive: 0.0,
            pbr_cheap: 0.0,
            pbr_fair: 0.0,
            pbr_expensive: 0.0,
            year_count: 0,
            index: 0,
        }
    }

    pub async fn upsert_all(date: NaiveDate, years: String) -> Result<PgQueryResult> {
        let sql = format!(
            r#"
WITH stocks AS (
    SELECT
        stock_symbol,
        last_four_eps,
		net_asset_value_per_share
    FROM
        stocks AS s
    WHERE
        s."SuspendListing" = false
),
price AS (
    SELECT
        "SecurityCode" AS stock_symbol,
        -- COUNT(DISTINCT "year") AS year_count,
        0 AS year_count,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY "LowestPrice") AS cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "ClosingPrice") AS fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY "HighestPrice") AS expensive
    FROM
        "DailyQuotes"
    WHERE
        "Date" <= '{1}'
        AND "year" IN ({0})
        AND "ClosingPrice" > 0
    GROUP BY
        "SecurityCode"
),
dividend AS (
    SELECT
        stock_symbol,
        dividend_base * 15 AS cheap,
        dividend_base * 20 AS fair,
        dividend_base * 25 AS expensive
    FROM
    (
        SELECT  stock_symbol, avg("sum") AS dividend_base
        FROM (
            SELECT
                security_code AS stock_symbol, sum("sum") AS sum
            FROM
                dividend
            WHERE
                "year" IN ({0}) and ("ex-dividend_date1" != '-' OR "ex-dividend_date2" != '-')
            GROUP BY
                security_code ,"year"
        ) AS inner_dividend
        GROUP BY stock_symbol
    ) AS calc
),
dividend_payout_ratio AS (
    SELECT
        security_code as stock_symbol,
        PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY d.payout_ratio) AS payout_ratio
    FROM
        public.dividend AS d
    WHERE
        d."year" IN ({0})
        AND d.payout_ratio > 0
		AND d.payout_ratio <= 100
    GROUP BY
        security_code
),
eps AS (
    SELECT
        stock_symbol,
        eps_base * 15 AS cheap,
        eps_base * 20 AS fair,
        eps_base * 25 AS expensive
    FROM
    (
        SELECT
            s.stock_symbol,
            CASE
                WHEN s.last_four_eps > 0 THEN s.last_four_eps * (dpr.payout_ratio / 100)
                ELSE 0
            END AS eps_base
        FROM
            stocks AS s
        INNER JOIN
            dividend_payout_ratio AS dpr ON s.stock_symbol = dpr.stock_symbol
    ) AS calc
),
pbr AS (
    SELECT
        calc.stock_symbol,
        cheap * s.net_asset_value_per_share AS cheap,
        fair * s.net_asset_value_per_share AS fair,
        expensive * s.net_asset_value_per_share AS expensive
    FROM
    (
        SELECT
            dq."SecurityCode" as stock_symbol,
            PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY "price-to-book_ratio") AS cheap,
            PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "price-to-book_ratio") AS fair,
            PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY "price-to-book_ratio") AS expensive
        FROM "DailyQuotes" AS dq
        WHERE
            "Date" <= '{1}'
            AND "year" IN ({0})
            AND "price-to-book_ratio" > 0
        GROUP BY dq."SecurityCode"
    ) AS calc
    INNER JOIN stocks as s on calc.stock_symbol = s.stock_symbol
),
per AS(
    SELECT
        calc.stock_symbol,
        dq.pe_high * calc.eps_avg as expensive,
        dq.pe_mid * calc.eps_avg as fair,
        dq.pe_low * calc.eps_avg as cheap
    FROM
    (
        SELECT
            inn_calc.stock_symbol,
            CASE
                WHEN AVG(eps) > 0 THEN AVG(eps)
                ELSE 0
            END AS eps_avg
            FROM
            (
                SELECT
                    fs.security_code AS stock_symbol,
                    fs.year,
                    SUM(fs.earnings_per_share) AS eps
                FROM financial_statement AS fs
                WHERE
                    "year" IN ({0})
                    AND quarter IN ('Q1','Q2','Q3','Q4')
                GROUP BY fs.security_code,fs.year
            ) AS inn_calc
        GROUP BY inn_calc.stock_symbol
    ) AS calc
    INNER JOIN
    (
        SELECT
            "SecurityCode" AS stock_symbol,
            PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY "PriceEarningRatio") AS pe_low,
            PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "PriceEarningRatio") AS pe_mid,
            PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY "PriceEarningRatio") AS pe_high
        FROM
            "DailyQuotes"
        WHERE
            "Date" <= '{1}'
            AND "year" IN ({0})
            AND "PriceEarningRatio" > 0
        GROUP BY
            "SecurityCode"
    ) AS dq on dq.stock_symbol = calc.stock_symbol
)
INSERT INTO estimate (
    security_code, "date", percentage, closing_price, cheap, fair, expensive, price_cheap,
    price_fair, price_expensive, dividend_cheap, dividend_fair, dividend_expensive, year_count,
    eps_cheap, eps_fair, eps_expensive, pbr_cheap, pbr_fair, pbr_expensive,
    per_cheap, per_fair, per_expensive, update_time
)
SELECT
    s.stock_symbol,
    dq."Date",
    (dq."ClosingPrice" / ((price.cheap * 0.2) + (dividend.cheap * 0.29) + (eps.cheap * 0.3) + (pbr.cheap * 0.2) + (per.cheap * 0.01))) * 100 AS percentage,
    dq."ClosingPrice",
    ((price.cheap * 0.2 ) + (dividend.cheap * 0.29) + (eps.cheap * 0.3) + (pbr.cheap * 0.2) + (per.cheap * 0.01)) AS cheap,
    ((price.fair * 0.2 ) + (dividend.fair * 0.29) + (eps.fair * 0.3) + (pbr.fair * 0.2) + (per.fair * 0.01)) AS fair,
    ((price.expensive * 0.2 ) + (dividend.expensive * 0.29) + (eps.expensive * 0.3) + (pbr.expensive * 0.2) + (per.expensive * 0.01)) AS expensive,
    price.cheap,
    price.fair,
    price.expensive,
    dividend.cheap,
    dividend.fair,
    dividend.expensive,
    year_count,
    eps.cheap,
    eps.fair,
    eps.expensive,
	pbr.cheap,
    pbr.fair,
    pbr.expensive,
    per.cheap,
    per.fair,
    per.expensive,
    NOW()
FROM stocks AS s
INNER JOIN "DailyQuotes" AS dq ON dq."SecurityCode" = s.stock_symbol AND dq."Date" = '{1}'
INNER JOIN price ON price.stock_symbol = s.stock_symbol
INNER JOIN dividend ON dividend.stock_symbol = s.stock_symbol
INNER JOIN eps ON eps.stock_symbol = s.stock_symbol
INNER JOIN pbr ON pbr.stock_symbol = s.stock_symbol
INNER JOIN per ON per.stock_symbol = s.stock_symbol
ON CONFLICT (date,security_code) DO UPDATE SET
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
	pbr_cheap = EXCLUDED.pbr_cheap,
    pbr_fair = EXCLUDED.pbr_fair,
    pbr_expensive = EXCLUDED.pbr_expensive,
    per_cheap = EXCLUDED.per_cheap,
    per_fair = EXCLUDED.per_fair,
    per_expensive = EXCLUDED.per_expensive,
    update_time = NOW();
"#,
            years, date
        );
        sqlx::query(&sql)
            .execute(database::get_connection())
            .await
            .context(format!("Failed to upsert_all() from database\nsql:{}", sql))
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
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY "LowestPrice") AS price_cheap,
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

),
pbr as (
    SELECT security_code,
           pbr_cheap * net_asset_value_per_share as pbr_cheap,
           pbr_fair * net_asset_value_per_share as pbr_fair,
           pbr_expensive * net_asset_value_per_share as pbr_expensive
    FROM (
        SELECT
            p.security_code_filter AS security_code,
            COALESCE(PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY "price-to-book_ratio"), 1) AS pbr_cheap,
            COALESCE(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "price-to-book_ratio"), 1) AS pbr_fair,
            COALESCE(PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY "price-to-book_ratio"), 1) AS pbr_expensive
        FROM params AS p
        LEFT JOIN "DailyQuotes" AS dq ON p.security_code_filter = dq."SecurityCode"
                                       AND "Date" <= p.date_filter
                                       AND "year" = ANY(p.year_filter)
                                       AND "price-to-book_ratio" > 0
        GROUP BY p.security_code_filter
    ) AS inner_pbr
    INNER JOIN stocks as s on inner_pbr.security_code = s.stock_symbol
),
per AS(
    SELECT
        calc.stock_symbol,
        dq.pe_high * calc.eps_avg as per_expensive,
        dq.pe_mid * calc.eps_avg as per_fair,
        dq.pe_low * calc.eps_avg as per_cheap
    FROM
    (
        SELECT
            inn_calc.stock_symbol,
            CASE
                WHEN AVG(eps) > 0 THEN AVG(eps)
                ELSE 0
            END AS eps_avg
            FROM
            (
                SELECT
                    fs.security_code AS stock_symbol,
                    fs.year,
                    SUM(fs.earnings_per_share) AS eps
                FROM params AS p
                INNER JOIN financial_statement AS fs ON p.security_code_filter = fs.security_code
                WHERE
                    "year" = ANY(p.year_filter)
                    AND quarter IN ('Q1','Q2','Q3','Q4')
                GROUP BY fs.security_code,fs.year
            ) AS inn_calc
        GROUP BY inn_calc.stock_symbol
    ) AS calc
    INNER JOIN (
        SELECT
            p.security_code_filter AS stock_symbol,
            COALESCE(PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY "PriceEarningRatio"), 0) AS pe_low,
            COALESCE(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "PriceEarningRatio"), 0) AS pe_mid,
            COALESCE(PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY "PriceEarningRatio"), 0) AS pe_high
        FROM params AS p
        LEFT JOIN "DailyQuotes" AS dq ON p.security_code_filter = dq."SecurityCode"
                                       AND "Date" <= p.date_filter
                                       AND "year" = ANY(p.year_filter)
                                       AND "PriceEarningRatio" > 0
        GROUP BY p.security_code_filter
    ) as dq on dq.security_code = calc.stock_symbol
)
INSERT INTO estimate (
    security_code, "date", percentage, closing_price, cheap, fair, expensive,
    price_cheap, price_fair, price_expensive,
    dividend_cheap, dividend_fair, dividend_expensive,
    eps_cheap, eps_fair, eps_expensive,
    pbr_cheap, pbr_fair, pbr_expensive,
    per_cheap, per_fair, per_expensive,
    year_count, update_time
)
SELECT
    p.security_code_filter,
    dq."Date",
    (dq."ClosingPrice" / ((price_cheap * 0.2 ) + (dividend_cheap * 0.29) + (eps_cheap * 0.3) + (pbr_cheap * 0.2) + (per_cheap * 0.01))) * 100 AS percentage,
    dq."ClosingPrice",
    ((price_cheap * 0.2 ) + (dividend_cheap * 0.29) + (eps_cheap * 0.3) + (pbr_cheap * 0.2) + (per_cheap * 0.01)) AS cheap,
    ((price_fair * 0.2 ) + (dividend_fair * 0.29) + (eps_fair * 0.3) + (pbr_fair * 0.2) + (per_fair * 0.01)) as fair,
    ((price_expensive * 0.2 ) + (dividend_expensive * 0.29)+ (eps_expensive * 0.3) + (pbr_expensive * 0.2) + (per_expensive * 0.01)) AS expensive,
    price_cheap,
    price_fair,
    price_expensive,
    dividend_cheap,
    dividend_fair,
    dividend_expensive,
    eps_cheap,
    eps_fair,
    eps_expensive,
    pbr_cheap,
    pbr_fair,
    pbr_expensive,
    per_cheap,
    per_fair,
    per_expensive,
    year_count,
    NOW()
FROM params AS p
INNER JOIN stocks AS s ON p.security_code_filter = s.stock_symbol
INNER JOIN "DailyQuotes" AS dq ON p.security_code_filter = dq."SecurityCode" and dq."Date" = p.date_filter
INNER JOIN price ON p.security_code_filter = price.security_code
INNER JOIN dividend ON p.security_code_filter = dividend.security_code
INNER JOIN eps ON p.security_code_filter = eps.stock_symbol
INNER JOIN pbr ON p.security_code_filter = pbr.security_code
INNER JOIN per ON p.security_code_filter = per.stock_symbol
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
    pbr_cheap = EXCLUDED.pbr_cheap,
    pbr_fair = EXCLUDED.pbr_fair,
    pbr_expensive = EXCLUDED.pbr_expensive,
    per_cheap = EXCLUDED.per_cheap,
    per_fair = EXCLUDED.per_fair,
    per_expensive = EXCLUDED.per_expensive,
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

        let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        let estimate = Estimate::new("9921".to_string(), current_date);

        match estimate.upsert(years_str).await {
            Ok(r) => logging::debug_file_async(format!("Estimate::upsert:{:#?}", r)),
            Err(why) => {
                logging::debug_file_async(format!("Failed to Estimate::upsert because {:?}", why));
            }
        }

        logging::debug_file_async("結束 Estimate::upsert".to_string());
    }

    #[tokio::test]
    async fn test_upsert_all() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 Estimate::upsert_all".to_string());

        let current_date = NaiveDate::parse_from_str("2023-10-20", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        match Estimate::upsert_all(current_date, years_str).await {
            Ok(r) => logging::debug_file_async(format!("Estimate::upsert_all:{:#?}", r)),
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to Estimate::upsert_all because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 Estimate::upsert_all".to_string());
    }
}
