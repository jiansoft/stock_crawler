use crate::internal::database::DB;
use anyhow::*;
use core::result::Result::Ok;
use sqlx::postgres::PgQueryResult;

#[derive(sqlx::FromRow, Default, Debug)]
/// 設定檔
pub struct Entity {
    pub key: String,
    pub val: String,
}

impl Entity {
    pub fn new(key: String) -> Self {
        Entity {
            key,
            ..Default::default()
        }
    }

    /// 取得一筆指定 key 的 Entity
    pub async fn first(key: &str) -> Result<Entity> {
        let sql = r#"
        SELECT key, val
        FROM config
        WHERE key = $1;
    "#;

        let entity = sqlx::query_as::<_, Entity>(sql)
            .bind(key)
            .fetch_one(&DB.pool)
            .await?;

        Ok(entity)
    }

    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
        INSERT INTO config (key, val)
        VALUES ($1, $2)
        ON CONFLICT (key)
        DO UPDATE SET val = excluded.val;"#;
        let result = sqlx::query(sql)
            .bind(&self.key)
            .bind(&self.val)
            .execute(&DB.pool)
            .await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;
    use chrono::{Local, NaiveDate};

    #[tokio::test]
    async fn test_first() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 first".to_string());
        let now = Local::now();
        let date_naive = now.date_naive();

        if let Ok(c) = Entity::first("last-closing-day").await {
            logging::debug_file_async(format!("last-closing-day:{:?}", c));
            let date = NaiveDate::parse_from_str(&c.val, "%Y-%m-%d").unwrap();

            logging::debug_file_async(format!("today:{:?}", date));
            logging::debug_file_async(format!("date_naive > date:{}", date_naive > date));
            if date_naive > date {
                let mut new_c = Entity::new(c.key);
                new_c.val = date_naive.format("%Y-%m-%d").to_string();
                match new_c.upsert().await {
                    Ok(result) => {
                        logging::debug_file_async(format!("upsert:{:#?}", result));
                    }
                    Err(why) => {
                        logging::debug_file_async(format!(
                            "Failed to config.upsert because:{:?}",
                            why
                        ));
                    }
                }
            }
        }

        logging::debug_file_async("結束 first".to_string());
    }
}
