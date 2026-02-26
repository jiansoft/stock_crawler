use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use sqlx::postgres::PgQueryResult;

use crate::database;

/// 系統設定表 `config` 的資料列。
#[derive(sqlx::FromRow, Default, Debug)]
pub struct Config {
    /// 設定鍵值名稱。
    pub key: String,
    /// 設定內容（以字串形式儲存）。
    pub val: String,
}

impl Config {
    /// 建立一筆 `Config` 實例。
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

    /// 新增或更新 `config` 的鍵值。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO config
    (key, val)
VALUES
    ($1, $2)
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

    /// 將 `val` 視為日期（`%Y-%m-%d`）並僅在新值較大時更新。
    ///
    /// 若資料庫已有同 `key` 設定且日期較新或相同，會回傳空的 `PgQueryResult` 表示略過更新。
    ///
    /// # Errors
    /// 當日期解析或 SQL 執行失敗時回傳錯誤。
    pub async fn set_val_as_naive_date(&self) -> Result<PgQueryResult> {
        let new_date = NaiveDate::parse_from_str(&self.val, "%Y-%m-%d")?;
        if let Ok(c) = Config::first(&self.key).await {
            let current_date = NaiveDate::parse_from_str(&c.val, "%Y-%m-%d")?;
            if new_date <= current_date {
                return Ok(PgQueryResult::default());
            }
        }

        self.upsert().await
    }

    /// 讀取目前 `key` 對應的值並解析為 `NaiveDate`。
    ///
    /// # Errors
    /// 當查無資料或日期格式不正確時回傳錯誤。
    pub async fn get_val_naive_date(&self) -> Result<NaiveDate> {
        if let Ok(c) = Config::first(&self.key).await {
            return Ok(NaiveDate::parse_from_str(&c.val, "%Y-%m-%d")?);
        }

        Err(anyhow!("can't use key({}) fine the value", self.key))
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;
    use chrono::{Local, NaiveDate};
    use std::result::Result::Ok;

    use super::*;

    #[tokio::test]
    #[ignore]
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
