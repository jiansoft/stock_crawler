use crate::{internal::crawler, internal::database::DB};
use chrono::Local;
use futures::StreamExt;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use sqlx::{self, FromRow};
use std::collections::HashMap;

#[derive(sqlx::Type, FromRow, Debug)]
#[sqlx(type_name = "index")]
pub struct Entity {
    pub category: String,
    pub date: chrono::NaiveDate,
    pub trade_value: Decimal,
    pub trading_volume: Decimal,
    pub transaction: Decimal,
    pub change: Decimal,
    pub index: Decimal,
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

    pub fn from_index_response(
        model: crawler::taiwan_capitalization_weighted_stock_index::Index,
    ) -> Self {
        Entity {
            category: model.category.to_string(),
            date: model.date,
            index: Decimal::from_f64(model.index).unwrap_or(Decimal::ZERO),
            change: Decimal::from_f64(model.change).unwrap_or(Decimal::ZERO),
            trade_value: Decimal::from_f64(model.trade_value).unwrap_or(Decimal::ZERO),
            transaction: Decimal::from_f64(model.transaction).unwrap_or(Decimal::ZERO),
            trading_volume: Decimal::from_f64(model.trading_volume).unwrap_or(Decimal::ZERO),
            create_time: model.create_time,
            update_time: model.update_time,
        }
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            category: self.category.to_string(),
            date: self.date.clone(),
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

pub async fn fetch() -> HashMap<String, Entity> {
    let stmt = r#"
select category, "date", trading_volume, "transaction", trade_value, change, index,create_time, update_time
from index
order by "date" desc
limit 30;
        "#;
    let mut stream = sqlx::query_as::<_, Entity>(&stmt).fetch(&DB.db);

    let mut indices: HashMap<String, Entity> = HashMap::new();

    while let Some(row_result) = stream.next().await {
        if let Ok(row) = row_result {
            let key = row.date.to_string() + "_" + row.category.as_str();
            indices.insert(key, row);
        };
    }

    indices
}

#[cfg(test)]
mod tests {
    use crate::logging;
    use super::*;

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        let r = fetch().await;
        for e in r.iter() {
            logging::info_file_async(format!("e.date {:?} e.index {:?}", e.1.date, e.1.index));
        }
        //logging::info_file_async(format!("結束"));
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
