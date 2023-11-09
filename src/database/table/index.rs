use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Local, NaiveDate};
use concat_string::concat_string;
use rust_decimal::Decimal;
use sqlx::{self, FromRow};

use crate::{
    util::map::Keyable,
    database,
    logging,
    util
};

#[derive(sqlx::Type, FromRow, Debug)]
pub struct Index {
    pub category: String,
    pub date: NaiveDate,
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

impl Index {
    pub fn new() -> Self {
        Index {
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

    pub async fn fetch() -> Result<Vec<Index>> {
        let sql: &str = r#"
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

        sqlx::query_as::<_, Index>(sql)
            .fetch_all(database::get_connection())
            .await
            .context(String::from("Failed to Index::fetch() from database"))
    }

    /// 將twse取回來的原始資料轉成 Entity
    pub fn from_strings(item: &[String]) -> Result<Self> {
        let split_date: Vec<&str> = item[0].split('/').collect();
        if split_date.len() != 3 {
            return Err(anyhow!("日期欄位不等於3"));
        }

        let year = split_date[0]
            .parse::<i32>()
            .map_err(|why| anyhow!(format!("轉換資料日期發生錯誤. because {:?}", why)))?;

        let mut index = Index::new();

        let date = concat_string!(
            util::datetime::roc_year_to_gregorian_year(year).to_string(),
            "-",
            split_date[1],
            "-",
            split_date[2]
        );

        index.date = NaiveDate::from_str(date.as_str())
            .map_err(|why| anyhow!(format!("Failed to parse date because {:?}", why)))?;

        index.trading_volume = util::text::parse_decimal(&item[1], None)?;
        index.trade_value = util::text::parse_decimal(&item[2], None)?;
        index.transaction = util::text::parse_decimal(&item[3], None)?;
        index.index = util::text::parse_decimal(&item[4], None)?;
        index.change = util::text::parse_decimal(&item[5], None)?;
        index.category = String::from("TAIEX");

        Ok(index)
    }

    /// date與 category 為組合鍵 unique
    pub async fn upsert(&self) -> Result<()> {
        let sql = r#"
INSERT INTO index
(
    category,
    "date",
    trading_volume,
    "transaction",
    trade_value,
    change,
    index,
    create_time,
    update_time
)
VALUES
(
    $1, $2, $3, $4, $5, $6, $7, $8, $9
)
ON CONFLICT
(
    "date", category
)
DO UPDATE
    SET update_time = EXCLUDED.update_time;
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
            .execute(database::get_connection())
            .await?;
        Ok(())
    }
}

impl Clone for Index {
    fn clone(&self) -> Self {
        Index {
            category: self.category.to_string(),
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

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<String>> for Index {
    fn from(item: Vec<String>) -> Self {
        let now = Local::now();
        let dy = util::datetime::gregorian_year_to_roc_year(now.year()).to_string();
        let dm = now.month().to_string();
        let dd = now.day().to_string();
        let mut split_date: Vec<&str> = item[0].split('/').collect();
        if split_date.len() != 3 {
            logging::error_file_async("日期欄位不等於3".to_string());
            split_date = vec![&dy, &dm, &dd]
        }

        let year = match split_date[0].parse::<i32>() {
            Ok(_year) => _year,
            Err(why) => {
                logging::error_file_async(format!("轉換資料日期發生錯誤. because {:?}", why));
                util::datetime::gregorian_year_to_roc_year(Local::now().year())
            }
        };

        let mut index = Index::new();
        index.category = String::from("TAIEX");
        let date = concat_string!(
            util::datetime::roc_year_to_gregorian_year(year).to_string(),
            "-",
            split_date[1],
            "-",
            split_date[2]
        );

        index.date = NaiveDate::from_str(date.as_str()).unwrap();
        /* let key = index.date.to_string() + "_" + &index.category;
        if let Ok(indices) = CACHE_SHARE.indices.read() {
            if indices.contains_key(key.as_str()) {
                continue;
            }
        }*/

        index.trading_volume = match Decimal::from_str(&item[1].replace(',', "")) {
            Ok(_trading_volume) => _trading_volume,
            Err(_) => Decimal::ZERO,
        };

        index.trade_value = match Decimal::from_str(&item[2].replace(',', "")) {
            Ok(_trade_value) => _trade_value,
            Err(_) => Decimal::ZERO,
        };

        index.transaction = match Decimal::from_str(&item[3].replace(',', "")) {
            Ok(_transaction) => _transaction,
            Err(_) => Decimal::ZERO,
        };

        index.index = match Decimal::from_str(&item[4].replace(',', "")) {
            Ok(_index) => _index,
            Err(_) => Decimal::ZERO,
        };

        index.change = match Decimal::from_str(&item[5].replace(',', "")) {
            Ok(_change) => _change,
            Err(_) => Decimal::ZERO,
        };
        index
    }
}

impl Keyable for Index {
    fn key(&self) -> String {
        format!("{}-{}", self.date, self.category)
    }

    fn key_with_prefix(&self) -> String {
        format!("Index:{}", self.key())
    }
}

#[cfg(test)]
mod tests {
    use std::time;

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_index_fetch() {
        dotenv::dotenv().ok();
        let r = Index::fetch().await.unwrap();
        for e in r.iter() {
            logging::info_file_async(format!("e.date {:?} e.index {:?}", e.date, e.index));
        }
        logging::info_file_async("結束".to_string());
        tokio::time::sleep(time::Duration::from_secs(1)).await;
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
