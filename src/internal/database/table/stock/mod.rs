pub(crate) mod extension;

use crate::internal::{
    crawler::{tpex, twse},
    database::{
        self,
        table::{stock::extension::StockJustWithSymbolAndName, stock_index, stock_word},
    },
    logging, util,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, postgres::PgRow, Row};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 原表名 stocks
pub struct Stock {
    pub stock_symbol: String,
    pub name: String,
    pub suspend_listing: bool,
    pub net_asset_value_per_share: Decimal,
    pub create_time: DateTime<Local>,
    /// 交易所的市場編號參考 stock_exchange_market
    pub stock_exchange_market_id: i32,
    /// 股票的產業分類編號 stock_industry
    pub stock_industry_id: i32,
}

impl Stock {
    pub fn new() -> Self {
        Stock {
            stock_symbol: "".to_string(),
            name: "".to_string(),
            suspend_listing: false,
            net_asset_value_per_share: Default::default(),
            create_time: Local::now(),
            stock_exchange_market_id: 0,
            stock_industry_id: 0,
        }
    }

    /// 是否為特別股
    pub fn is_preference_shares(&self) -> bool {
        is_preference_shares(&self.stock_symbol)
    }

    /// 是否為臺灣存託憑證
    pub fn is_tdr(&self) -> bool {
        self.name.contains("-DR")
    }

    /// 更新個股最新一季與近四季的EPS
    pub async fn update_last_eps() -> Result<PgQueryResult> {
        let sql = r#"
with eps_data as (
	select row_number() OVER (PARTITION BY security_code order by year desc,quarter desc) AS row_number, serial
	from financial_statement
	where year in ($1,$2) and quarter in ('Q1', 'Q2', 'Q3', 'Q4')
),
eps_row as (
	select row_number, fs.security_code, fs.earnings_per_share
	from financial_statement fs
	inner join eps_data on fs.serial = eps_data.serial
),
last_one_eps as (select security_code, earnings_per_share from eps_row where row_number = 1),
last_four_eps as (
	select eps_row.security_code, sum(eps_row.earnings_per_share) as eps
	from eps_row
	where eps_row.row_number in (1, 2, 3, 4)
	group by eps_row.security_code
),
eps as (
	select last_four_eps.security_code, eps as last_four_eps, last_one_eps.earnings_per_share as last_one_eps
	from last_four_eps
	inner join last_one_eps on last_four_eps.security_code = last_one_eps.security_code
)
update stocks Set last_four_eps = eps.last_four_eps,last_one_eps = eps.last_one_eps
FROM eps
where eps.security_code = stocks.stock_symbol;
"#;
        let now = Local::now();
        let one_year_ago = now - Duration::days(365);
        Ok(sqlx::query(sql)
            .bind(now.year())
            .bind(one_year_ago.year())
            .execute(database::get_pool()?)
            .await?)
    }

    /// 更新個股的每股淨值
    pub async fn update_net_asset_value_per_share(&self) -> Result<PgQueryResult> {
        let sql = r#"
update
    stocks
set
    net_asset_value_per_share = $2
where
    stock_symbol = $1;
"#;
        sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.net_asset_value_per_share)
            .execute(database::get_pool()?)
            .await
            .context("Failed to update net_asset_value_per_share")
    }

    pub async fn update_suspend_listing(&self) -> Result<PgQueryResult> {
        let sql = r#"
update
    stocks
set
    "SuspendListing" = $2
where
    stock_symbol = $1;
"#;
        Ok(sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.suspend_listing)
            .execute(database::get_pool()?)
            .await?)
    }

    /// 衝突時更新 "Name" "SuspendListing" stock_exchange_market_id stock_industry_id
    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO stocks (
    stock_symbol, "Name", "CreateTime",
    "SuspendListing", stock_exchange_market_id, stock_industry_id)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (stock_symbol) DO UPDATE SET
    "Name" = EXCLUDED."Name",
    "SuspendListing" = EXCLUDED."SuspendListing",
    stock_exchange_market_id = EXCLUDED.stock_exchange_market_id,
    stock_industry_id = EXCLUDED.stock_industry_id;
"#;
        let result = sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(&self.name)
            .bind(self.create_time)
            .bind(self.suspend_listing)
            .bind(self.stock_exchange_market_id)
            .bind(self.stock_industry_id)
            .execute(database::get_pool()?)
            .await?;
        self.create_index().await;
        Ok(result)
    }

    async fn create_index(&self) {
        // 拆解股票名稱為單詞並加入股票代碼
        let mut words = util::text::split(&self.name);
        words.push(self.stock_symbol.to_string());

        // 查詢已存在的單詞，轉成 hashmap 方便查詢
        let words_in_db = stock_word::StockWord::list_by_word(&words).await;
        let exist_words = stock_word::vec_to_hashmap_key_using_word(words_in_db.ok());

        for word in words {
            let mut stock_index_e = stock_index::StockIndex::new(self.stock_symbol.to_string());

            match exist_words.get(&word) {
                Some(w) => {
                    //word 已存在資料庫了
                    stock_index_e.word_id = w.word_id;
                }
                None => {
                    let mut stock_word_e = stock_word::StockWord::new(word);
                    match stock_word_e.upsert().await {
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

    /*    /// 依照指定的年月取得該股票其月份的最低、平均、最高價
        pub async fn lowest_avg_highest_price_by_year_and_month(
            &self,
            year: i32,
            month: i32,
        ) -> Result<(Decimal, Decimal, Decimal)> {
            let sql = r#"
    SELECT
        MIN("LowestPrice"),
        AVG("ClosingPrice"),
        MAX("HighestPrice")
    FROM "DailyQuotes"
    WHERE "SecurityCode" = $1 AND year = $2 AND month = $3
    GROUP BY "SecurityCode", year, month;
    "#;
            let (lowest_price, avg_price, highest_price) =
                sqlx::query_as::<_, (Decimal, Decimal, Decimal)>(sql)
                    .bind(&self.stock_symbol)
                    .bind(year)
                    .bind(month)
                    .fetch_one(&DB.pool)
                    .await?;

            Ok((lowest_price, avg_price, highest_price))
        }*/

    /// 取得所有股票
    pub async fn fetch() -> Result<Vec<Stock>> {
        let sql = r#"
SELECT
    stock_symbol,
    "Name" AS name,
    "SuspendListing" AS suspend_listing,
    "CreateTime" AS create_time,
    net_asset_value_per_share,
    stock_exchange_market_id,
    stock_industry_id
FROM
    stocks
ORDER BY
    stock_exchange_market_id,
    stock_industry_id;
"#;
        let answers = sqlx::query(sql)
            .try_map(|row: PgRow| {
                Ok(Stock {
                    stock_symbol: row.try_get("stock_symbol")?,
                    net_asset_value_per_share: row.try_get("net_asset_value_per_share")?,
                    name: row.try_get("name")?,
                    suspend_listing: row.try_get("suspend_listing")?,
                    create_time: row.try_get("create_time")?,
                    stock_exchange_market_id: row.try_get("stock_exchange_market_id")?,
                    stock_industry_id: row.try_get("stock_industry_id")?,
                })
            })
            .fetch_all(database::get_pool()?)
            .await?;

        Ok(answers)
    }
}

impl Clone for Stock {
    fn clone(&self) -> Self {
        Stock {
            stock_symbol: self.stock_symbol.clone(),
            name: self.name.clone(),
            suspend_listing: self.suspend_listing,
            net_asset_value_per_share: self.net_asset_value_per_share,
            create_time: self.create_time,
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_industry_id: self.stock_industry_id,
        }
    }
}

impl Default for Stock {
    fn default() -> Self {
        Stock::new()
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber> for Stock {
    fn from(isin: twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber) -> Self {
        Stock {
            stock_symbol: isin.stock_symbol,
            name: isin.name,
            suspend_listing: false,
            net_asset_value_per_share: Default::default(),
            create_time: Local::now(),
            stock_exchange_market_id: isin.exchange_market.stock_exchange_market_id,
            stock_industry_id: isin.industry_id,
        }
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<tpex::net_asset_value_per_share::Emerging> for Stock {
    fn from(tpex: tpex::net_asset_value_per_share::Emerging) -> Self {
        Stock {
            stock_symbol: tpex.stock_symbol,
            name: "".to_string(),
            suspend_listing: false,
            net_asset_value_per_share: tpex.net_asset_value_per_share,
            create_time: Local::now(),
            stock_exchange_market_id: Default::default(),
            stock_industry_id: Default::default(),
        }
    }
}

/// 取得未下市每股淨值為零的股票
pub async fn fetch_net_asset_value_per_share_is_zero() -> Result<Vec<Stock>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    s.stock_exchange_market_id,
    s.stock_industry_id
FROM stocks AS s
WHERE s.stock_exchange_market_id in (2, 4)
    AND s."SuspendListing" = false
    AND s.net_asset_value_per_share = 0
"#;

    Ok(sqlx::query_as::<_, Stock>(sql)
        .fetch_all(database::get_pool()?)
        .await?)
}

/// 取得尚未有指定年度的季報的股票或者財報的每股淨值為零的股票
pub async fn fetch_stocks_without_financial_statement(
    year: i32,
    quarter: &str,
) -> Result<Vec<Stock>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    stock_exchange_market_id,
    stock_industry_id
FROM stocks AS s
WHERE s.stock_exchange_market_id in(2, 4)
    AND s."SuspendListing" = false
    AND
(
    NOT EXISTS (
        SELECT 1
        FROM financial_statement f
        WHERE f.security_code = s.stock_symbol AND f.year = $1 AND f.quarter = $2
    )
    OR EXISTS (
        SELECT 1
        FROM financial_statement f
        WHERE f.security_code = s.stock_symbol AND f.year = $1 AND f.quarter = $2 and f.net_asset_value_per_share = 0
    )
)
"#;

    Ok(sqlx::query_as::<_, Stock>(sql)
        .bind(year)
        .bind(quarter)
        .fetch_all(database::get_pool()?)
        .await?)
}

/// 取得指定日期為除息權日的股票
pub async fn fetch_stocks_with_dividends_on_date(
    date: NaiveDate,
) -> Result<Vec<StockJustWithSymbolAndName>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name
FROM
    dividend AS d
INNER JOIN
    stocks AS s ON s.stock_symbol = d.security_code
WHERE
    d."year" = $1
    AND (d."ex-dividend_date1" = $2 OR d."ex-dividend_date2" = $2);
"#;

    let year = date.year();
    let date_str = date.format("%Y-%m-%d").to_string();

    Ok(sqlx::query_as::<_, StockJustWithSymbolAndName>(sql)
        .bind(year)
        .bind(&date_str)
        .fetch_all(database::get_pool()?)
        .await?)
}

/// 是否為特別股
pub fn is_preference_shares(stock_symbol: &str) -> bool {
    stock_symbol
        .chars()
        .any(|c| c.is_ascii_uppercase() || c.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;
    use chrono::TimeZone;

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        //logging::info_file_async("開始 fetch".to_string());
        let r = Stock::fetch().await;
        if let Ok(result) = r {
            for e in result {
                logging::info_file_async(format!("{:#?} ", e));
            }
        }
        //logging::info_file_async("結束 fetch".to_string());
    }

    /*    #[tokio::test]
    async fn test_fetch_avg_lowest_highest_price() {
        dotenv::dotenv().ok();
        //logging::info_file_async("開始 fetch".to_string());
        let mut e = Stock::new();
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

    }*/

    #[tokio::test]
    async fn test_fetch_net_asset_value_per_share_is_zero() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_net_asset_value_per_share_is_zero".to_string());
        match fetch_net_asset_value_per_share_is_zero().await {
            Ok(stocks) => {
                for e in stocks {
                    logging::debug_file_async(format!("{} {:?} ", e.is_preference_shares(), e));
                }
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_net_asset_value_per_share_is_zero because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_net_asset_value_per_share_is_zero".to_string());
    }

    #[tokio::test]
    async fn test_fetch_stocks_without_financial_statement() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stocks_without_financial_statement".to_string());
        match fetch_stocks_without_financial_statement(2022, "Q4").await {
            Ok(stocks) => {
                for e in stocks {
                    logging::debug_file_async(format!("{} {:?} ", e.is_preference_shares(), e));
                }
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_stocks_without_financial_statement because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_stocks_without_financial_statement".to_string());
    }

    #[tokio::test]
    async fn test_fetch_stocks_with_specified_ex_dividend_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stocks_with_specified_ex_dividend_date".to_string());

        let ex_date = Local.with_ymd_and_hms(2023, 4, 20, 0, 0, 0).unwrap();
        let d = ex_date.date_naive();
        match fetch_stocks_with_dividends_on_date(d).await {
            Ok(cd) => {
                logging::debug_file_async(format!("stock: {:?}", cd));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch_stocks_with_specified_ex_dividend_date".to_string());
    }

    #[tokio::test]
    async fn test_create_index() {
        dotenv::dotenv().ok();
        let mut e = Stock::new();
        e.stock_symbol = "2330".to_string();
        e.name = "台積電".to_string();
        e.create_index().await;
    }
}
