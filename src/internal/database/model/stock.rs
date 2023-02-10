use crate::{internal::database::DB};
use chrono::{DateTime, Local};
use sqlx::{postgres::PgRow, Error, Row};
use sqlx::postgres::PgQueryResult;


#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct Entity {
    pub category: i32,
    pub security_code: String,
    pub name: String,
    pub create_time: DateTime<Local>,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            category: Default::default(),
            security_code: Default::default(),
            name: Default::default(),
            create_time: Local::now(),
        }
    }

    /*/// 建立一個 Entity 數據來源為 international_securities_identification_number::Stock
    pub fn from_isin_response(
        model: &crawler::international_securities_identification_number::Stock,
    ) -> Self {
        Entity {
            category: model.category,
            security_code: model.security_code.to_string(),
            name: model.name.to_string(),
            create_time: model.create_time,
        }
    }*/

    pub async fn upsert(&self) -> Result<PgQueryResult, Error> {
        let sql = r#"
insert into "Company" (
    "SecurityCode", "Name", "CategoryId", "CreateTime", "SuspendListing"
) values (
    $1,$2,$3,$4,false
) on conflict ("SecurityCode") do nothing;
        "#;
        sqlx::query(sql)
            .bind(self.security_code.as_str())
            .bind(self.name.as_str())
            .bind(self.category)
            .bind(self.create_time)
            .execute(&DB.pool)
            .await
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            category: self.category,
            security_code: self.security_code.clone(),
            name: self.name.clone(),
            create_time: self.create_time,
        }
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new()
    }
}

/*pub async fn fetch() -> HashMap<String, Entity> {
    let stmt = r#"
select "CategoryId","SecurityCode","Name","CreateTime"
FROM "Company"
;
        "#;

    let mut stocks: HashMap<String, Entity> = HashMap::new();

    let mut stream = sqlx::query(stmt).fetch(&DB.pool);
    while let Some(row_result) = stream.next().await {
        if let Ok(row) = row_result {
            // let q:Result<String> =row.try_get("SecurityCode");
            // let create_time = ;
            let mut stock = Entity::new();
            stock.create_time = match row.try_get::<DateTime<Local>, &str>("CreateTime") {
                Ok(time) => time,
                Err(why) => {
                    println!("why {:#?} ", why);
                    Default::default()
                }
            };

            stock.security_code = match row.try_get::<&str, &str>("SecurityCode") {
                Ok(s) => s.to_string(),
                Err(why) => {
                    logging::error_file_async(format!("why {:#?} ", why));
                    "".to_string()
                }
            };
            stock.name = row.try_get("Name").unwrap_or("".to_string());
            stock.category = row.try_get("CategoryId").unwrap_or(0);
            //logging::info_file_async(format!("stock {:#?} ", stock));
        }
    }
    stocks
}*/

pub async fn fetch() -> Result<Vec<Entity>, Error> {
    let answers = sqlx::query(
        r#"
        select "CategoryId","SecurityCode","Name","CreateTime"
        from "Company"
        order by "CategoryId"
        "#,
    )
    .try_map(|row: PgRow| {
        let category = row.try_get("CategoryId")?;
        let security_code = row.try_get("SecurityCode")?;
        let name = row.try_get("Name")?;
        let create_time = row.try_get("CreateTime")?;
        Ok(Entity {
            category,
            security_code,
            name,
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


    /*    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        logging::info_file_async(format!("開始 Company"));
        let r = fetch().await;
        logging::info_file_async(format!("len:{}", r.len()));
        for e in r.iter() {
            logging::info_file_async(format!(
                "e.security_code {:?} e.name {:?}",
                e.1.security_code, e.1.name
            ));
        }
        logging::info_file_async(format!("結束"));
        //thread::sleep(time::Duration::from_secs(1));
    }*/

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 fetch".to_string());
        let r = fetch().await;
        if let Ok(result) = r {
            for e in result {
                logging::info_file_async(format!(
                    "e.security_code {:?} e.name {:?}",
                    e.security_code, e.name
                ));
            }
        }
    }
}
