use crate::internal::database;
use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use sqlx::postgres::PgQueryResult;

#[derive(sqlx::FromRow, Debug, Default)]
pub struct Estimate {
    pub date: NaiveDate, // 使用 chrono 庫來處理日期和時間
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
    pub fn new() -> Self {
        Estimate {
            ..Default::default()
        }
    }
    pub async fn rebuild() -> Result<PgQueryResult> {
        let sql = r#"

"#;
        let year_ago = Local::now() - Duration::days(365);
        sqlx::query(sql)
            .bind(year_ago)
            .execute(database::get_connection())
            .await
            .context("Failed to rebuild from database")
    }
}
