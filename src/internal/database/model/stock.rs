use crate::{
    internal::database::model::stock_index, internal::database::model::stock_word,
    internal::database::DB, internal::util, logging,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{postgres::PgRow, Row};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 原表名 stocks
pub struct Entity {
    pub category: i32,
    pub stock_symbol: String,
    pub name: String,
    pub suspend_listing: bool,
    pub net_asset_value_per_share: Decimal,
    pub create_time: DateTime<Local>,
}

impl Entity {
    pub fn new() -> Self {
        Entity {
            create_time: Local::now(),
            ..Default::default()
        }
    }

    /// 檢查股票是否為特別股
    pub fn is_preference_shares(&self) ->  bool{
        self.stock_symbol.chars().any(|c| c.is_ascii_uppercase())
    }

    pub async fn update_suspend_listing(&self) -> Result<()> {
        let sql = r#"
update
    stocks
set
    "SuspendListing" = $2
where
    stock_symbol = $1;
"#;

        sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.suspend_listing)
            .execute(&DB.pool)
            .await
            .map_err(|err| anyhow!("Failed to update suspend listing: {:?}", err))?;
        Ok(())
    }

    pub async fn upsert(&self) -> Result<()> {
        let sql = r#"
        INSERT INTO stocks (stock_symbol, "Name", "CategoryId", "CreateTime", "SuspendListing")
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (stock_symbol) DO UPDATE SET
        "Name" = EXCLUDED."Name",
        "CategoryId" = EXCLUDED."CategoryId",
        "SuspendListing" = EXCLUDED."SuspendListing";
    "#;
        sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(&self.name)
            .bind(self.category)
            .bind(self.create_time)
            .bind(self.suspend_listing)
            .execute(&DB.pool)
            .await
            .map_err(|err| anyhow!("Failed to stock upsert: {:?}", err))?;
        self.create_index().await;
        Ok(())
    }

    async fn create_index(&self) {
        //32,市認售 33,指數類 31,市認購
        //166,櫃認售 165,櫃認購
        //51,市牛證 52,市熊證
        match self.category {
            31 | 32 | 33 | 51 | 52 | 165 | 166 => return,
            _ => {}
        }

        // 拆解股票名稱為單詞並加入股票代碼
        let mut words = util::text::split(&self.name);
        words.push(self.stock_symbol.to_string());

        // 查詢已存在的單詞，轉成 hashmap 方便查詢
        let words_in_db = stock_word::Entity::list_by_word(&words).await;
        let exist_words = stock_word::vec_to_hashmap_key_using_word(words_in_db);

        for word in words {
            let mut stock_index_e = stock_index::Entity::new(self.stock_symbol.to_string());

            match exist_words.get(&word) {
                Some(w) => {
                    //word 已存在資料庫了
                    stock_index_e.word_id = w.word_id;
                }
                None => {
                    let mut stock_word_e = stock_word::Entity::new(word);
                    match stock_word_e.insert().await {
                        Ok(word_id) => {
                            stock_index_e.word_id = word_id;
                        }
                        Err(why) => {
                            logging::error_file_async(format!(
                                "Failed to insert stock word because:{:#?}",
                                why
                            ));
                            continue;
                        }
                    }
                }
            }

            if let Err(why) = stock_index_e.insert().await {
                logging::error_file_async(format!(
                    "Failed to insert stock index because:{:#?}",
                    why
                ));
            }
        }
    }

    /// 依照指定的年月取得該股票其月份的最低、平均、最高價
    pub async fn lowest_avg_highest_price_by_year_and_month(
        &self,
        year: i32,
        month: i32,
    ) -> Result<(Decimal, Decimal, Decimal)> {
        /*let answers = sqlx::query(
        r#"
        SELECT
            MIN("LowestPrice") AS lowest_price,
            AVG("ClosingPrice") AS avg_price,
            MAX("HighestPrice") AS highest_price
        FROM "DailyQuotes"
        WHERE "SecurityCode" = $1 AND year = $2 AND month = $3
        GROUP BY "SecurityCode", year, month
        "#,
                )
                .bind(&self.stock_symbol)
                .bind(year)
                .bind(month)
                .try_map(|row: PgRow| {
                    let lowest_price: Decimal = row.try_get("lowest_price")?;
                    let avg_price: Decimal = row.try_get("avg_price")?;
                    let highest_price: Decimal = row.try_get("highest_price")?;
                    Ok((lowest_price, avg_price, highest_price))
                })
                .fetch_one(&DB.pool)
                .await?;

                Ok(answers)*/
        let (lowest_price, avg_price, highest_price) =
            sqlx::query_as::<_, (Decimal, Decimal, Decimal)>(
                r#"
                SELECT
                    MIN("LowestPrice"),
                    AVG("ClosingPrice"),
                    MAX("HighestPrice")
                FROM "DailyQuotes"
                WHERE "SecurityCode" = $1 AND year = $2 AND month = $3
                GROUP BY "SecurityCode", year, month;
                "#,
            )
            .bind(&self.stock_symbol)
            .bind(year)
            .bind(month)
            .fetch_one(&DB.pool)
            .await?;

        Ok((lowest_price, avg_price, highest_price))
    }
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            category: self.category,
            stock_symbol: self.stock_symbol.clone(),
            name: self.name.clone(),
            suspend_listing: self.suspend_listing,
            net_asset_value_per_share: self.net_asset_value_per_share,
            create_time: self.create_time,
        }
    }

    //todo 抓完季報後需要更新 stocks.net_asset_value_per_share 的數據
}

impl Default for Entity {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn fetch() -> Result<Vec<Entity>> {
    let answers = sqlx::query(
        r#"
select
    "CategoryId", stock_symbol, "Name", "SuspendListing", "CreateTime",
    net_asset_value_per_share
from
    stocks
order by
    "CategoryId"
"#,
    )
    .try_map(|row: PgRow| {
        Ok(Entity {
            stock_symbol: row.try_get("stock_symbol")?,
            net_asset_value_per_share: row.try_get("net_asset_value_per_share")?,
            name: row.try_get("Name")?,
            category: row.try_get("CategoryId")?,
            suspend_listing: row.try_get("SuspendListing")?,
            create_time: row.try_get("CreateTime")?,
        })
    })
    .fetch_all(&DB.pool)
    .await?;

    Ok(answers)
}

pub async fn fetch_net_asset_value_per_share_is_zero() -> Result<Vec<Entity>> {
    let rows = sqlx::query_as::<_, Entity>(
        r#"
select
    s."CategoryId" as category, s.stock_symbol, s."Name" as name,
    s."SuspendListing" as suspend_listing, s."CreateTime" as create_time,
    s.net_asset_value_per_share
from market_category as mc
inner join category c on mc.market_category_id = c.market_category_id
inner join stocks as s on c.category_id = s."CategoryId"
where mc.market_category_id in (2,4)
    and  c.category_id IN (
        1, 2, 3, 4, 6, 7, 9, 10, 11, 12, 13, 19, 20, 21, 22, 24,
        30, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 121, 122,
        123, 124, 125, 126, 130, 131, 138, 139, 140, 141, 142, 145,
        151, 153, 154, 155, 156, 157, 158, 159, 160, 161, 169, 170, 171)
    and s."SuspendListing" = false and net_asset_value_per_share = 0
"#,
    )
    .fetch_all(&DB.pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        //logging::info_file_async("開始 fetch".to_string());
        let r = fetch().await;
        if let Ok(result) = r {
            for e in result {
                logging::info_file_async(format!("{:#?} ", e));
            }
        }
        //logging::info_file_async("結束 fetch".to_string());
    }

    #[tokio::test]
    async fn test_fetch_avg_lowest_highest_price() {
        dotenv::dotenv().ok();
        //logging::info_file_async("開始 fetch".to_string());
        let mut e = Entity::new();
        e.stock_symbol = String::from("2402");
        match e.lowest_avg_highest_price_by_year_and_month(2023, 3).await {
            Ok((lowest_price, avg_price, highest_price)) => {
                logging::info_file_async(format!(
                    "stock_symbol:{} lowest_price:{} avg_price:{} highest_price:{}",
                    e.stock_symbol, lowest_price, avg_price, highest_price
                ));
            }
            Err(why) => {
                logging::error_file_async(format!("{:#?}", why));
            }
        }
        /*let r = e.lowest_avg_highest_price_by_year_and_month(2023, 3).await;
        if let Ok((lowest_price, avg_price, highest_price)) = r {
            logging::info_file_async(format!(
                "lowest_price:{} avg_price:{} highest_price:{}",
                lowest_price, avg_price, highest_price
            ));
        }*/
    }

    #[tokio::test]
    async fn test_fetch_net_asset_value_per_share_is_zero() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 fetch_net_asset_value_per_share_is_zero".to_string());
        match fetch_net_asset_value_per_share_is_zero().await {
            Ok(stocks) => {
                for e in stocks {
                    logging::info_file_async(format!("{} {:?} ",e.is_preference_shares(), e));
                }
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to fetch_net_asset_value_per_share_is_zero because: {:?}",
                    why
                ));
            }
        }

        logging::info_file_async("結束 fetch_net_asset_value_per_share_is_zero".to_string());
    }

    #[tokio::test]
    async fn test_create_index() {
        dotenv::dotenv().ok();
        let mut e = Entity::new();
        e.stock_symbol = "2330".to_string();
        e.name = "台積電".to_string();
        e.create_index().await;
    }
}
