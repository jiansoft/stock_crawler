use crate::{
    internal::database::model::stock_index, internal::database::model::stock_word,
    internal::database::DB, logging,
};
use anyhow::Result;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, postgres::PgRow, Error, Row};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct Entity {
    pub category: i32,
    pub security_code: String,
    pub name: String,
    pub suspend_listing: bool,
    pub create_time: DateTime<Local>,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            category: Default::default(),
            security_code: Default::default(),
            name: Default::default(),
            suspend_listing: false,
            create_time: Local::now(),
        }
    }

    pub async fn update_suspend_listing(&self) -> Result<PgQueryResult, Error> {
        let sql = r#"
update
    "Company"
set
    "SuspendListing" = $2
where
    "SecurityCode" = $1;
"#;

        sqlx::query(sql)
            .bind(self.security_code.as_str())
            .bind(self.suspend_listing)
            .execute(&DB.pool)
            .await
    }

    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
insert into "Company" (
    "SecurityCode", "Name", "CategoryId", "CreateTime", "SuspendListing"
) values (
    $1,$2,$3,$4,false
) on conflict ("SecurityCode") do nothing;
"#;
        let result = sqlx::query(sql)
            .bind(self.security_code.as_str())
            .bind(self.name.as_str())
            .bind(self.category)
            .bind(self.create_time)
            .execute(&DB.pool)
            .await?;
        self.create_index().await;
        Ok(result)
    }

    async fn create_index(&self) {
        //32,市認售 33,指數類 31,市認購
        //166,櫃認售 165,櫃認購
        //51,市牛證 52,市熊證
        if self.category == 31
            || self.category == 32
            || self.category == 33
            || self.category == 51
            || self.category == 52
            || self.category == 165
            || self.category == 166
        {
            return;
        }

        let mut words = stock_word::split(self.name.as_str());
        words.push(self.security_code.to_string());
        let word_in_db = stock_word::fetch_by_word(&words).await;

        for word in words {
            let mut stock_index_e = stock_index::Entity::new(self.security_code.to_string());

            match word_in_db.get(&word) {
                Some(w) => {
                    //word 已存在資料庫了
                    stock_index_e.word_id = w.word_id;
                }
                None => {
                    let mut stock_word_e = stock_word::Entity::new(word);
                    match stock_word_e.insert().await {
                        Ok(word_id) => {
                            stock_index_e.word_id = word_id;
                        }
                        Err(why) => {
                            logging::error_file_async(format!("because:{:#?}", why));
                            continue;
                        }
                    }
                }
            }

            match stock_index_e.insert().await {
                Ok(()) => {}
                Err(why) => {
                    logging::error_file_async(format!("because:{:#?}", why));
                }
            }
        }
    }

    /// 依照指定的年月取得該股票其月份的最低、平均、最高價
    pub async fn lowest_avg_highest_price_by_year_and_month(
        &self,
        year: i32,
        month: i32,
    ) -> Result<(Decimal, Decimal, Decimal)> {
        let answers = sqlx::query(
            r#"
select
    min("LowestPrice") as lowest_price,
    avg("ClosingPrice") as avg_price,
    max("HighestPrice") as highest_price
from "DailyQuotes"
where
    "SecurityCode" = $1 and year = $2 and month = $3
group by "SecurityCode", year, month

        "#,
        )
        .bind(self.security_code.as_str())
        .bind(year)
        .bind(month)
        .try_map(|row: PgRow| {
            let lowest_price: Decimal = row.try_get("lowest_price")?;
            let avg_price: Decimal = row.try_get("avg_price")?;
            let highest_price: Decimal = row.try_get("highest_price")?;
            Ok((lowest_price, avg_price, highest_price))
        })
        .fetch_one(&DB.pool)
        .await?;

        Ok(answers)
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            category: self.category,
            security_code: self.security_code.clone(),
            name: self.name.clone(),
            suspend_listing: self.suspend_listing,
            create_time: self.create_time,
        }
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn fetch() -> Result<Vec<Entity>, Error> {
    let answers = sqlx::query(
        r#"
        select "CategoryId","SecurityCode","Name", "SuspendListing", "CreateTime"
        from "Company"
        order by "CategoryId"
        "#,
    )
    .try_map(|row: PgRow| {
        let category = row.try_get("CategoryId")?;
        let security_code = row.try_get("SecurityCode")?;
        let name = row.try_get("Name")?;
        let suspend_listing = row.try_get("SuspendListing")?;
        let create_time = row.try_get("CreateTime")?;
        Ok(Entity {
            category,
            security_code,
            name,
            suspend_listing,
            create_time,
        })
    })
    .fetch_all(&DB.pool)
    .await;

    answers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        //logging::info_file_async("開始 fetch".to_string());
        let r = fetch().await;
        if let Ok(result) = r {
            for e in result {
                logging::info_file_async(format!("{:#?} ", e));
            }
        }
    }

    #[tokio::test]
    async fn test_fetch_avg_lowest_highest_price() {
        dotenv::dotenv().ok();
        //logging::info_file_async("開始 fetch".to_string());
        let mut e = Entity::new();
        e.security_code = String::from("1101");
        let r = e.lowest_avg_highest_price_by_year_and_month(2023, 1).await;
        if let Ok((lowest_price, avg_price, highest_price)) = r {
            logging::info_file_async(format!(
                "lowest_price:{} avg_price:{} highest_price:{}",
                lowest_price, avg_price, highest_price
            ));
        }
    }
    #[tokio::test]
    async fn test_create_index() {
        dotenv::dotenv().ok();
        let mut e = Entity::new();
        e.security_code = "2330".to_string();
        e.name = "台積電".to_string();
        e.create_index().await;
    }
}
