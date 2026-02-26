use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use sqlx::{postgres::PgQueryResult, Postgres, Transaction};

use crate::database;

/// 股票與關鍵字索引關聯表資料列（`company_index`）。
#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct StockIndex {
    /// 關鍵字表 `company_word.word_id`。
    pub word_id: i64,
    /// 股票代號。
    pub security_code: String,
    /// 建立時間。
    pub created_time: DateTime<Local>,
    /// 最後更新時間。
    pub updated_time: DateTime<Local>,
}

impl StockIndex {
    /// 建立指定股票代號的索引實例。
    pub fn new(security_code: String) -> Self {
        StockIndex {
            word_id: Default::default(),
            security_code,
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    /// 刪除指定股票代號的所有關鍵字索引。
    ///
    /// # Errors
    /// 當 transaction 或 SQL 執行失敗時回傳錯誤。
    pub async fn delete_by_stock_symbol(stock_symbol: &str) -> Result<PgQueryResult> {
        let mut transaction: Transaction<Postgres> = database::get_tx().await?;
        match sqlx::query("DELETE FROM company_index WHERE security_code = $1;")
            .bind(stock_symbol)
            .execute(&mut *transaction)
            .await
        {
            Ok(r) => {
                transaction.commit().await?;
                Ok(r)
            }
            Err(why) => {
                transaction.rollback().await?;
                Err(anyhow!(
                    "Failed to delete_by_stock_symbol because: {:?}",
                    why
                ))
            }
        }
    }

    /// 寫入一筆股票關鍵字索引（衝突時忽略）。
    ///
    /// # Errors
    /// 當 `word_id <= 0` 或 SQL 執行失敗時回傳錯誤。
    pub async fn insert(&self) -> Result<()> {
        if self.word_id <= 0 {
            return Err(anyhow!("word_id is less than or equal to 0"));
        }

        let mut transaction: Transaction<Postgres> = database::get_tx().await?;

        if let Err(why) = sqlx::query(
            "
INSERT INTO
    company_index (
        word_id,
        security_code,
        created_time,
        updated_time
    )
VALUES
    (
        $1,
        $2,
        $3,
        $4
    )
ON CONFLICT
    (word_id, security_code)
DO NOTHING;
",
        )
        .bind(self.word_id)
        .bind(&self.security_code)
        .bind(self.created_time)
        .bind(self.updated_time)
        .execute(&mut *transaction)
        .await
        {
            transaction.rollback().await?;
            return Err(anyhow!(
                "Failed to insert into company_index because: {:?}",
                why
            ));
        }

        transaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_insert() {
        dotenv::dotenv().ok();
        let mut e = StockIndex::new("79979".to_string());
        e.word_id = 79979;
        match e.insert().await {
            Ok(_) => {
                match sqlx::query_as::<sqlx::Postgres, (i64, )>("select count(*) as row_count from company_index where word_id = $1 and security_code = $2;")
                    .bind(e.word_id)
                    .bind(e.security_code.as_str())
                    .fetch_one(database::get_connection())
                    .await
                {
                    Ok((row_count, )) => {
                        logging::info_file_async(format!("row_count:{}", row_count));
                        let _ = sqlx::query(
                            "delete from company_index where word_id = $1 and security_code = $2;",
                        )
                            .bind(e.word_id)
                            .bind(e.security_code.as_str())
                            .execute(database::get_connection())
                            .await;
                    }
                    Err(why) => {
                        logging::error_file_async(format!("because:{:#?}", why));
                    }
                };
            }
            Err(why) => {
                logging::error_file_async(format!("because:{:#?}", why));
            }
        }
    }
}
