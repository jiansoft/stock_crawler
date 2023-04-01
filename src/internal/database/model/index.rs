use crate::{internal::database::DB, logging};
use anyhow;
use chrono::Local;
use futures::StreamExt;
use rust_decimal::Decimal;
use sqlx::{self, FromRow};
use std::collections::HashMap;

#[derive(sqlx::Type, FromRow, Debug)]
pub struct Entity {
    pub category: String,
    pub date: chrono::NaiveDate,
    pub index: Decimal,
    /// 漲跌點數
    pub change: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    pub create_time: chrono::DateTime<Local>,
    pub update_time: chrono::DateTime<Local>,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            category: Default::default(),
            date: Default::default(),
            index: Default::default(),
            change: Default::default(),
            trade_value: Default::default(),
            transaction: Default::default(),
            trading_volume: Default::default(),
            create_time: Local::now(),
            update_time: Local::now(),
        }
    }

    /// date與 category 為組合鍵 unique
    pub async fn upsert(&self) -> anyhow::Result<()> {
        let sql = r#"
insert into index (
    category, "date", trading_volume, "transaction", trade_value, change, index, create_time, update_time
) values (
    $1,$2,$3,$4,$5,$6,$7,$8,$9
) ON CONFLICT ("date",category) DO UPDATE SET update_time = excluded.update_time;
        "#;
        sqlx::query(sql)
            .bind(&self.category)
            .bind(self.date)
            .bind(self.trading_volume)
            .bind(self.transaction)
            .bind(self.trade_value)
            .bind(self.change)
            .bind(self.index)
            .bind(self.create_time)
            .bind(self.update_time)
            .execute(&DB.pool)
            .await?;
        Ok(())
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            category: self.category.clone(),
            date: self.date,
            trade_value: self.trade_value,
            trading_volume: self.trading_volume,
            transaction: self.transaction,
            change: self.change,
            index: self.index,
            create_time: self.create_time,
            update_time: self.create_time,
        }
    }
}

impl Default for Entity {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn fetch() -> anyhow::Result<HashMap<String, Entity>> {
    const STMT: &str = r#"
        SELECT
            category,
            "date",
            trading_volume,
            "transaction",
            trade_value,
            change,
            index,
            create_time,
            update_time
        FROM
            index
        ORDER BY
            "date" DESC
        LIMIT 30;
    "#;

    let mut stream = sqlx::query_as::<_, Entity>(STMT).fetch(&DB.pool);

    let mut indices = HashMap::with_capacity(30);

    while let Some(row_result) = stream.next().await {
        match row_result {
            Ok(row) => {
                let key = format!("{}_{}", row.date, row.category);
                indices.insert(key, row);
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to stream.next() because {:?}", why));
            }
        };
    }

    Ok(indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;
    use std::{thread, time};

    #[tokio::test]
    async fn test_index_fetch() {
        dotenv::dotenv().ok();
        let r = fetch().await.unwrap();
        for e in r.iter() {
            logging::info_file_async(format!("e.date {:?} e.index {:?}", e.1.date, e.1.index));
        }
        logging::info_file_async("結束".to_string());
        thread::sleep(time::Duration::from_secs(1));
        /* while let Some(result) = fetch().await.next().await {
            if let Ok(ref row_result) = result {
                logging::info_file_async(format!(
                    "row.date {:?} row.index {:?}",
                    row_result.date, row_result.index
                ));
                /*if let Ok(row) = row_result {
                    logging::info_file_async(format!("row.date {:?} row.index {:?}", row.date, row.index));
                    //indices.insert(row.date.to_string(),row);
                };*/
            }
        }*/
    }
}
