use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Datelike, Local, TimeDelta};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, postgres::PgRow, Row};

use crate::{
    crawler::{tpex, twse},
    database::{
        self,
        table::{stock_index, stock_word},
    },
    logging,
    util::{self, map::Keyable},
};

pub(crate) mod extension;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 原表名 stocks
pub struct Stock {
    pub stock_symbol: String,
    pub name: String,
    pub suspend_listing: bool,
    pub net_asset_value_per_share: Decimal,
    // 權植佔比
    pub weight: Decimal,
    // 股東權益報酬率
    pub return_on_equity: Decimal,
    pub create_time: DateTime<Local>,
    /// 交易所的市場編號參考 StockExchangeMarket
    pub stock_exchange_market_id: i32,
    /// 股票的產業分類編號 stock_industry
    pub stock_industry_id: i32,
    /// 已發行股數
    pub issued_share: i64,
    /// 全體外資及陸資持有股數
    pub qfii_shares_held: i64,
    /// 全體外資及陸資持股比率
    pub qfii_share_holding_percentage: Decimal,
}

impl Stock {
    pub fn new() -> Self {
        Stock {
            stock_symbol: "".to_string(),
            name: "".to_string(),
            suspend_listing: false,
            net_asset_value_per_share: Default::default(),
            weight: Default::default(),
            return_on_equity: Default::default(),
            create_time: Local::now(),
            stock_exchange_market_id: 0,
            stock_industry_id: 0,
            issued_share: 0,
            qfii_shares_held: 0,
            qfii_share_holding_percentage: Default::default(),
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

    /// 更新個股最新一季、近四季的EPS、ROE
    pub async fn update_eps_and_roe() -> Result<PgQueryResult> {
        let sql = r#"
WITH fs_data AS (
    SELECT
        row_number() OVER (
            PARTITION BY security_code
            ORDER BY year DESC, quarter DESC
        ) AS row_number,
        serial
    FROM
        financial_statement
    WHERE
        year IN ($1, $2)
        AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
),
relevant_fs_rows AS (
    SELECT
        fs_data.row_number,
        fs.security_code,
        fs.earnings_per_share,
        fs.net_asset_value_per_share,
        fs.return_on_equity
    FROM
        financial_statement fs
    JOIN
        fs_data ON fs_data.serial = fs.serial
    ORDER BY year DESC, quarter DESC
),
aggregated_eps AS (
    SELECT
        security_code,
        SUM(earnings_per_share) AS last_four_eps
    FROM
        relevant_fs_rows
    WHERE
        row_number <= 4
    GROUP BY
        security_code
)
UPDATE
    stocks
SET
    last_four_eps = agg.last_four_eps,
    last_one_eps = current_row.earnings_per_share,
    net_asset_value_per_share = current_row.net_asset_value_per_share,
    return_on_equity = current_row.return_on_equity
FROM
    relevant_fs_rows AS current_row
JOIN
    aggregated_eps AS agg ON current_row.security_code = agg.security_code
WHERE
    current_row.security_code = stocks.stock_symbol;
"#;
        let now = Local::now();
        let one_year_ago = now - TimeDelta::try_days(365).unwrap();

        sqlx::query(sql)
            .bind(now.year())
            .bind(one_year_ago.year())
            .execute(database::get_connection())
            .await
            .context("Failed to update_last_eps from database")
    }

    /// 衝突時更新 "Name" "SuspendListing" stock_exchange_market_id stock_industry_id
    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO stocks (
    stock_symbol, "Name", "CreateTime",
    "SuspendListing", stock_exchange_market_id, stock_industry_id,weight)
VALUES ($1, $2, $3, $4, $5, $6, 0)
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
            .execute(database::get_connection())
            .await
            .context("Failed to stock.upsert from database");
        self.create_index().await;

        result
    }

    async fn create_index(&self) {
        // 先刪除舊的數據
        if let Err(why) = stock_index::StockIndex::delete_by_stock_symbol(&self.stock_symbol).await
        {
            logging::error_file_async(format!("{:#?}", why));
        }

        // 拆解股票名稱為單詞並加入股票代碼
        let mut words = util::text::split(&self.name);
        words.push(self.stock_symbol.to_string());

        // 查詢已存在的單詞，轉成 hashmap 方便查詢
        let words_in_db = stock_word::StockWord::list_by_word(&words).await;
        let exist_words = match words_in_db {
            Ok(sw) => util::map::vec_to_hashmap(sw),
            Err(why) => {
                logging::error_file_async(format!("Failed to list_by_word because:{:#?}", why));
                return;
            }
        };

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

    /// 取得所有股票
    pub async fn fetch() -> Result<Vec<Stock>> {
        let sql = r#"
SELECT
    stock_symbol,
    "Name" AS name,
    "SuspendListing" AS suspend_listing,
    "CreateTime" AS create_time,
    net_asset_value_per_share,
    return_on_equity,
    weight,
    stock_exchange_market_id,
    stock_industry_id,
    issued_share,
    qfii_shares_held,
    qfii_share_holding_percentage
FROM
    stocks
ORDER BY
    stock_exchange_market_id,
    stock_industry_id;
"#;
        sqlx::query(sql)
            .try_map(|row: PgRow| {
                Ok(Stock {
                    stock_symbol: row.try_get("stock_symbol")?,
                    net_asset_value_per_share: row.try_get("net_asset_value_per_share")?,
                    weight: row.try_get("weight")?,
                    name: row.try_get("name")?,
                    suspend_listing: row.try_get("suspend_listing")?,
                    create_time: row.try_get("create_time")?,
                    stock_exchange_market_id: row.try_get("stock_exchange_market_id")?,
                    stock_industry_id: row.try_get("stock_industry_id")?,
                    issued_share: row.try_get("issued_share")?,
                    qfii_shares_held: row.try_get("qfii_shares_held")?,
                    return_on_equity: row.try_get("return_on_equity")?,
                    qfii_share_holding_percentage: row.try_get("qfii_share_holding_percentage")?,
                })
            })
            .fetch_all(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to Stock::fetch from database({:#?}) because:{:?}",
                    crate::config::SETTINGS.postgresql,
                    why
                )
            })
    }
}

impl Keyable for Stock {
    fn key(&self) -> String {
        self.stock_symbol.clone()
    }

    fn key_with_prefix(&self) -> String {
        format!("Stock:{}", self.key())
    }
}

impl Clone for Stock {
    fn clone(&self) -> Self {
        Stock {
            stock_symbol: self.stock_symbol.clone(),
            name: self.name.clone(),
            suspend_listing: self.suspend_listing,
            net_asset_value_per_share: self.net_asset_value_per_share,
            weight: self.weight,
            return_on_equity: self.return_on_equity,
            create_time: self.create_time,
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_industry_id: self.stock_industry_id,
            issued_share: self.issued_share,
            qfii_shares_held: self.qfii_shares_held,
            qfii_share_holding_percentage: self.qfii_share_holding_percentage,
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
            weight: Default::default(),
            return_on_equity: Default::default(),
            create_time: Local::now(),
            stock_exchange_market_id: isin.exchange_market.stock_exchange_market_id,
            stock_industry_id: isin.industry_id,
            issued_share: 0,
            qfii_shares_held: 0,
            qfii_share_holding_percentage: Default::default(),
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
            weight: Default::default(),
            return_on_equity: Default::default(),
            create_time: Local::now(),
            stock_exchange_market_id: Default::default(),
            stock_industry_id: Default::default(),
            issued_share: 0,
            qfii_shares_held: 0,
            qfii_share_holding_percentage: Default::default(),
        }
    }
}

/// 取得未下市上市櫃每股淨值為零的股票
pub async fn fetch_net_asset_value_per_share_is_zero() -> Result<Vec<Stock>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    s.return_on_equity,
    s.stock_exchange_market_id,
    s.stock_industry_id,
    s.weight,
    s.issued_share,
    s.qfii_shares_held,
    s.qfii_share_holding_percentage
FROM stocks AS s
WHERE s.stock_exchange_market_id in (2, 4)
    AND s."SuspendListing" = false
    AND s.net_asset_value_per_share = 0
"#;

    sqlx::query_as::<_, Stock>(sql)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_net_asset_value_per_share_is_zero from database")
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
    s.return_on_equity,
    s.stock_exchange_market_id,
    s.stock_industry_id,
    s.weight,
    s.issued_share,
    s.qfii_shares_held,
    s.qfii_share_holding_percentage
FROM stocks AS s
WHERE s.stock_exchange_market_id in(2, 4)
    AND s."SuspendListing" = false
    AND NOT EXISTS (
        SELECT 1
        FROM financial_statement f
        WHERE f.security_code = s.stock_symbol AND f.year = $1 AND f.quarter = $2
    )
"#;

    sqlx::query_as::<_, Stock>(sql)
        .bind(year)
        .bind(quarter)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_stocks_without_financial_statement from database")
}

/// 是否為特別股
pub fn is_preference_shares(stock_symbol: &str) -> bool {
    stock_symbol
        .chars()
        .any(|c| c.is_ascii_uppercase() || c.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_update_last_eps() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 update_last_eps".to_string());
        match Stock::update_eps_and_roe().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to update_last_eps because: {:?}", why));
            }
        }

        logging::debug_file_async("結束 update_last_eps".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 Stock::fetch".to_string());
        match Stock::fetch().await {
            Ok(stocks) => {
                logging::debug_file_async(format!("stocks:{:#?}", stocks));
            }
            Err(why) => {
                logging::debug_file_async(format!("{:?}", why));
            }
        }
        logging::debug_file_async("結束 Stock::fetch".to_string());
    }

    #[tokio::test]
    #[ignore]
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
    #[ignore]
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
    #[ignore]
    async fn test_create_index() {
        dotenv::dotenv().ok();
        let mut e = Stock::new();
        e.stock_symbol = "2330".to_string();
        e.name = "台積電".to_string();
        e.create_index().await;
    }
}
