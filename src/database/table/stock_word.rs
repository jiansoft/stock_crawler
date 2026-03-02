use anyhow::Result;
use chrono::{DateTime, Local};
use sqlx::{postgres::PgRow, QueryBuilder, Row};

use crate::{database, util::map::Keyable};

#[rustfmt::skip]
/// 股票搜尋關鍵字資料列（`company_word`）。
#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct StockWord {
    /// 關鍵字主鍵。
    pub word_id: i64,
    /// 關鍵字內容。
    pub word: String,
    /// 建立時間。
    pub created_time: DateTime<Local>,
    /// 最後更新時間。
    pub updated_time: DateTime<Local>,
}

impl StockWord {
    /// 建立單一關鍵字實例。
    pub fn new(word: String) -> Self {
        StockWord {
            word_id: Default::default(),
            word,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 複製目前實例內容。
    pub fn clone(&self) -> Self {
        StockWord {
            word_id: self.word_id,
            word: self.word.to_string(),
            created_time: self.created_time,
            updated_time: self.updated_time,
        }
    }

    /// 新增數據到資料庫後回傳新增的 word_id
    pub async fn upsert(&mut self) -> Result<i64> {
        let sql = "
INSERT INTO company_word (word, created_time, updated_time)
VALUES ($1, $2, $3)
ON CONFLICT (word) DO UPDATE SET
    updated_time = EXCLUDED.updated_time
RETURNING word_id";

        let row = sqlx::query(sql)
            .bind(&self.word)
            .bind(self.created_time)
            .bind(self.updated_time)
            .fetch_one(database::get_connection())
            .await?;

        self.word_id = row.try_get("word_id")?;

        Ok(self.word_id)
    }

    /// 從資料表中取得公司代碼、名字拆字後的數據
    pub async fn list_by_word(words: &Vec<String>) -> Result<Vec<StockWord>> {
        let mut query_builder =
            QueryBuilder::new("select word_id,word,created_time,updated_time from company_word");

        if !words.is_empty() {
            query_builder.push(" where word = any(");
            query_builder.push_bind(words);
            query_builder.push(")");
        }

        Ok(query_builder
            .build()
            .try_map(|row: PgRow| {
                let created_time = row.try_get("created_time")?;
                let updated_time = row.try_get("updated_time")?;
                let word_id = row.try_get("word_id")?;
                let word = row.try_get("word")?;
                Ok(StockWord {
                    word_id,
                    word,
                    created_time,
                    updated_time,
                })
            })
            .fetch_all(database::get_connection())
            .await?)
    }
}

impl Clone for StockWord {
    fn clone(&self) -> Self {
        self.clone()
    }
}

impl Default for StockWord {
    fn default() -> Self {
        Self::new("".to_string())
    }
}

impl Keyable for StockWord {
    fn key(&self) -> String {
        self.word.clone()
    }

    fn key_with_prefix(&self) -> String {
        format!("StockWord:{}", self.key())
    }
}

/*/// 將 vec 轉成 hashmap
pub fn vec_to_hashmap_key_using_word(
    entities: Option<Vec<StockWord>>,
) -> HashMap<String, StockWord> {
    let mut stock_words = HashMap::new();
    if let Some(list) = entities {
        for e in list {
            stock_words.insert(e.word.to_string(), e);
        }
    }

    stock_words
}*/

/*/// 將 vec 轉成 hashmap
fn vec_to_hashmap(v: Option<Vec<Entity>>) -> HashMap<String, Entity> {
    v.unwrap_or_default()
        .iter()
        .fold(HashMap::new(), |mut acc, e| {
            acc.insert(e.word.to_string(), e.clone());
            acc
        })
}*/

#[cfg(test)]
mod tests {
    use crate::{logging, util};

    use super::*;

    /*    #[tokio::test]
        async fn test_vec_to_hashmap() {
            dotenv::dotenv().ok();
            let mut entities: Vec<StockWord> = Vec::new();
            for i in 0..1000000 {
                entities.push(StockWord {
                    word_id: 0,
                    word: format!("word_{}", i),
                    created_time: Default::default(),
                    updated_time: Default::default(),
                });
            }

            let start1 = Instant::now();
            let _hm1 = vec_to_hashmap_key_using_word(Some(entities.clone()));
            let elapsed1 = start1.elapsed().as_millis();

            /*let start2 = Instant::now();
            let hm2 = vec_to_hashmap(Some(entities.clone()));
            let elapsed2 = start2.elapsed().as_millis();*/

            println!("Method 1 elapsed time: {}", elapsed1);
            //println!("Method 2 elapsed time: {}", elapsed2);
            //println!("HashMap length: {} {}", hm1.len(), hm2.len());
        }
    */
    /*    #[tokio::test]
        async fn test_split_1() {
            dotenv::dotenv().ok();
            let chinese_word = "台積電";
            let start = Instant::now();
            let result = split_v1(chinese_word);
            let end = start.elapsed();
            println!("split: {:?}, elapsed time: {:?}", result, end);
        }
    */

    #[tokio::test]
    #[ignore]
    async fn test_insert() {
        dotenv::dotenv().ok();
        let mut e = StockWord::new("小一".to_string());
        match e.upsert().await {
            Ok(word_id) => {
                logging::debug_file_async(format!("word_id:{} e:{:#?}", word_id, &e));
                let _ = sqlx::query("delete from company_word where word_id = $1;")
                    .bind(word_id)
                    .execute(database::get_connection())
                    .await;
            }
            Err(why) => {
                logging::debug_file_async(format!("because:{:?}", why));
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_by_word() {
        dotenv::dotenv().ok();
        let word = util::text::split("隆銘綠能");
        let entities = StockWord::list_by_word(&word).await;
        logging::debug_file_async(format!("entities:{:#?}", entities));
        /*logging::debug_file_async(format!(
            "word:{:#?}",
            util::map::vec_to_hashmap(entities.unwrap())
        ));*/
    }
}
