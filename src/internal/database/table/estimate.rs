use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
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
WITH price as (
	SELECT
        "SecurityCode",
        COUNT(DISTINCT "year") AS year_count,
        min("ClosingPrice") AS price_cheap,
        AVG("ClosingPrice") AS price_fair,
        max("ClosingPrice") AS price_expensive
    FROM
        "DailyQuotes"
    WHERE
        "SecurityCode" = $1 and
        "year" in ({0})
        AND "ClosingPrice" > 0
    GROUP BY
        "SecurityCode"
),
dividend as (
	SELECT
        security_code,
        AVG("sum") * 15 AS dividend_cheap,
        AVG("sum") * 20 AS dividend_fair,
        AVG("sum") * 30 AS dividend_expensive
    FROM
        dividend
    WHERE
        security_code = $1
        AND "year" IN ({0})
    GROUP BY
        security_code
)
insert into estimate (
	security_code, "date", percentage, closing_price, cheap, fair, expensive, price_cheap,
	price_fair, price_expensive, dividend_cheap, dividend_fair, dividend_expensive, year_count
)
select dq."SecurityCode",
	dq."Date",
	(((dq."ClosingPrice" / ((price_cheap + dividend_cheap) / 2))) * 100) as percentage,
	dq."ClosingPrice",
	(price_cheap + dividend_cheap) / 2                                   as cheap,
	(price_fair + dividend_fair) / 2                                     as fair,
	(price_expensive + dividend_expensive) / 2                           as expensive,
	price_cheap,
	price_fair,
	price_expensive,
	dividend_cheap,
	dividend_fair,
	dividend_expensive,
	year_count
from stocks AS c
inner join "DailyQuotes" as dq on "c".stock_symbol = dq."SecurityCode" and dq."Date" = $2
inner join price on dq."SecurityCode" = price."SecurityCode"
inner join dividend on dq."SecurityCode" = dividend.security_code
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
    year_count = EXCLUDED.year_count;
"#,
            years
        );

        sqlx::query(&sql)
            .bind(&self.security_code)
            .bind(self.date)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to upsert estimate({:#?}) from database",
                self
            ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use crate::internal::cache::SHARE;
    use crate::internal::logging;

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
