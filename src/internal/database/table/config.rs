use anyhow::*;
use sqlx::postgres::PgQueryResult;

use crate::internal::database;

#[derive(sqlx::FromRow, Default, Debug)]
/// 設定檔
pub struct Config {
    pub key: String,
    pub val: String,
}

impl Config {
    pub fn new(key: String, val: String) -> Self {
        Config { key, val }
    }

    /// 取得一筆指定 key 的 Entity
    pub async fn first(key: &str) -> Result<Config> {
        let sql = r#"
        SELECT key, val
        FROM config
        WHERE key = $1;
    "#;

        sqlx::query_as::<_, Config>(sql)
            .bind(key)
            .fetch_one(database::get_connection())
            .await
            .context(format!("Failed to Config::first({:?}) from database", key))
    }

    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
        INSERT INTO config (key, val)
        VALUES ($1, $2)
        ON CONFLICT (key)
        DO UPDATE SET val = excluded.val;"#;
        sqlx::query(sql)
            .bind(&self.key)
            .bind(&self.val)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to Config::upsert({:#?}) from database",
                self
            ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate};

    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_first() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 first".to_string());
        let now = Local::now();
        let date_naive = now.date_naive();

        if let Ok(c) = Config::first("last-closing-day").await {
            logging::debug_file_async(format!("last-closing-day:{:?}", c));
            let date = NaiveDate::parse_from_str(&c.val, "%Y-%m-%d").unwrap();

            logging::debug_file_async(format!("today:{:?}", date));
            logging::debug_file_async(format!("date_naive > date:{}", date_naive > date));
            if date_naive > date {
                let new_c = Config::new(c.key, date_naive.format("%Y-%m-%d").to_string());
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
