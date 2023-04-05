use chrono::NaiveDate;
use sqlx::{
    postgres::PgRow,
    Row
};


use crate::internal::database::{DB, model};


use crate::logging;

/// 提醒本日為除權息的股票有那些
pub async fn execute(date: NaiveDate) {
    match  sqlx::query(
        r#"
select s.stock_symbol,s."Name"
from dividend as d
inner join stocks as s on s.stock_symbol = d.security_code
where "ex-dividend_date1" = $1 or "ex-dividend_date2" = $2
        "#,
    )
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(date.format("%Y-%m-%d").to_string())
        .try_map(|row: PgRow| {
            Ok(model::stock::Entity {
                category: 0,
                stock_symbol: row.try_get("stock_symbol")?,
                name: row.try_get("Name")?,
                suspend_listing: false,
                create_time: Default::default(),
            })
        })
        .fetch_all(&DB.pool).await{
        Ok(stocks) => {
            logging::info_file_async(format!("date:{:?} rows:{:#?}", date,stocks));
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to fetch entities (model::stock) because: {:?}",
                why
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        execute(date.unwrap()).await;

        logging::info_file_async("結束 execute".to_string());
    }
}
