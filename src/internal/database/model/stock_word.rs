use crate::{internal::database::DB, logging};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use sqlx::{postgres::PgRow, Postgres, QueryBuilder, Row};
use std::collections::HashMap;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct Entity {
    pub word_id: i64,
    pub word: String,
    pub created_time: DateTime<Local>,
    pub updated_time: DateTime<Local>,
}

impl Entity {
    pub fn new(word: String) -> Self {
        Entity {
            word_id: Default::default(),
            word,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    pub fn clone(&self) -> Self {
        Entity {
            word_id: self.word_id,
            word: self.word.to_string(),
            created_time: self.created_time,
            updated_time: self.updated_time,
        }
    }

    pub async fn insert(&mut self) -> Result<i64> {
        let mut transaction = DB.pool.begin().await?;
        match sqlx::query_as::<Postgres, (i64, )>("insert into company_word (word, created_time, updated_time) VALUES ($1,$2,$3) RETURNING word_id;")
            .bind(self.word.as_str())
            .bind(self.created_time)
            .bind(self.updated_time)
            .fetch_one(&mut transaction)
            .await
        {
            Ok((last_insert_id, )) => {
                transaction.commit().await?;
                self.word_id = last_insert_id;
                Ok(last_insert_id)
            }
            Err(why) => {
                transaction.rollback().await?;
                Err(anyhow!("{:?}", why))
            }
        }
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        self.clone()
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new("".to_string())
    }
}

/// 從資料表中取得公司代碼、名字拆字後的數據
pub async fn fetch_by_word(words: &Vec<String>) -> HashMap<String, Entity> {
    let mut stock_words: HashMap<String, Entity> = HashMap::new();
    if words.is_empty() {
        return stock_words;
    }

    let mut query_builder: QueryBuilder<Postgres> = QueryBuilder::new(
        "
select
    word_id,word,created_time,updated_time
from
    company_word
where
    word IN (",
    );
    let mut separated = query_builder.separated(", ");

    for value_type in words.iter() {
        separated.push_bind(value_type);
    }
    separated.push_unseparated(")");

    //builder.push(" AND channel_id=").push_bind(channel);

    let query = query_builder.build();

    match query
        .try_map(|row: PgRow| {
            let word_id = row.try_get("word_id")?;
            let word = row.try_get("word")?;
            let created_time = row.try_get("created_time")?;
            let updated_time = row.try_get("updated_time")?;
            Ok(Entity {
                word_id,
                word,
                created_time,
                updated_time,
            })
        })
        .fetch_all(&DB.pool)
        .await
    {
        Ok(result) => {
            for e in result {
                stock_words.insert(e.word.to_string(), e);
            }
        }
        Err(why) => {
            logging::error_file_async(format!("because:{:#?}", why));
        }
    }

    stock_words
}

/// 將中文字拆分 例︰台積電 => ["台", "台積", "台積電", "積", "積電", "電"]
pub fn split(w: &str) -> Vec<String> {
    let word = w.replace('*', "");
    let word = word.replace('=', "");
    let mut words = Vec::new();
    let words_chars: Vec<char> = word.chars().collect();
    let words_len = words_chars.len();

    for i in 0..(words_len) {
        let mut s = String::from("");
        let first_word = words_chars[i].to_string();
        s += first_word.as_str();

        if !words.contains(&first_word) {
            words.push(first_word);
        }

        for (index, c) in words_chars.iter().enumerate() {
            if index <= i {
                continue;
            }

            s += c.to_string().as_str();
            if !words.contains(&s) {
                words.push(s.to_string());
            }
        }

        let w = s.to_string();
        if !words.contains(&w) {
            words.push(w);
        }
    }

    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_split() {
        dotenv::dotenv().ok();
        let r = split("台積電");
        println!("{:?}", r)
    }

    #[tokio::test]
    async fn test_insert() {
        dotenv::dotenv().ok();
        let mut e = Entity::new("小一".to_string());
        match e.insert().await {
            Ok(word_id) => {
                logging::info_file_async(format!("word_id:{} e:{:#?}", word_id, &e));
                let _ = sqlx::query("delete from company_word where word_id = $1;")
                    .bind(word_id)
                    .execute(&DB.pool)
                    .await;
            }
            Err(why) => {
                logging::error_file_async(format!("because:{:?}", why));
            }
        }
    }

    #[tokio::test]
    async fn test_fetch_stock_word() {
        dotenv::dotenv().ok();
        let word = split("台積電");
        logging::info_file_async(format!("word:{:#?}", fetch_by_word(&word).await));
    }
}
