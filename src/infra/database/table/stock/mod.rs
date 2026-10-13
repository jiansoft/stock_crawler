use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Local, TimeDelta};
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgQueryResult, postgres::PgRow};

use crate::{
    core::declare::Industry,
    core::util::{self, map::Keyable},
    infra::database,
};

pub(crate) mod extension;
pub mod stock_exchange_market;
pub(crate) mod stock_index;
pub(crate) mod stock_word;

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone)]
/// 原表名 stocks
pub struct StockDbRow {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 是否下市或停止交易。
    pub suspend_listing: bool,
    /// 每股淨值。
    pub net_asset_value_per_share: Decimal,
    /// 權值占比。
    pub weight: Decimal,
    /// 股東權益報酬率（ROE）。
    pub return_on_equity: Decimal,
    /// 建立時間。
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

impl StockDbRow {
    /// 建立 `StockDbRow` 預設值。
    pub fn new() -> Self {
        StockDbRow {
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

    /// 取得所有股票
    pub async fn fetch() -> Result<Vec<StockDbRow>> {
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
                Ok(StockDbRow {
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
                    "Failed to StockDbRow::fetch from database({}) because:{:?}",
                    database::redacted_postgresql_summary(),
                    why
                )
            })
    }

    /// 依現有 `stocks` 主檔重建搜尋索引。
    ///
    /// 用於直接執行 migration 匯入資料後，補建 `company_word` / `company_index`。
    pub async fn rebuild_search_indices() -> Result<()> {
        for stock in Self::fetch().await? {
            create_search_index(&stock.stock_symbol, &stock.name).await;
        }

        Ok(())
    }
}

/// <summary>
/// 為特定證券主檔建立或重新建立搜尋索引。
/// </summary>
pub async fn create_search_index(stock_symbol: &str, name: &str) {
    // 先刪除舊的數據
    if let Err(why) = stock_index::StockIndex::delete_by_stock_symbol(stock_symbol).await {
        tracing::error!("{:#?}", why);
    }

    // 拆解股票名稱為單詞並加入股票代碼
    let mut words = util::text::split(name);
    words.push(stock_symbol.to_string());

    // 查詢已存在的單詞，轉成 hashmap 方便查詢
    let words_in_db = stock_word::StockWord::list_by_word(&words).await;
    let exist_words = match words_in_db {
        Ok(sw) => util::map::vec_to_hashmap(sw),
        Err(why) => {
            tracing::error!("Failed to list_by_word because:{:#?}", why);
            return;
        }
    };

    for word in words {
        let mut stock_index_e = stock_index::StockIndex::new(stock_symbol.to_string());

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
                        tracing::error!("Failed to insert stock word because:{:#?}", why);
                        continue;
                    }
                }
            }
        }

        if let Err(why) = stock_index_e.insert().await {
            tracing::error!("Failed to insert stock index because:{:#?}", why);
        }
    }
}

impl Keyable for StockDbRow {
    fn key(&self) -> String {
        self.stock_symbol.clone()
    }

    fn key_with_prefix(&self) -> String {
        format!("Stock:{}", self.key())
    }
}

impl Default for StockDbRow {
    fn default() -> Self {
        StockDbRow::new()
    }
}

impl From<StockDbRow> for crate::domain::registry::entity::Stock {
    fn from(row: StockDbRow) -> Self {
        crate::domain::registry::entity::Stock::reconstitute(
            row.stock_symbol,
            row.name,
            row.suspend_listing,
            row.net_asset_value_per_share,
            row.weight,
            row.return_on_equity,
            row.create_time,
            row.stock_exchange_market_id,
            row.stock_industry_id,
            row.issued_share,
            row.qfii_shares_held,
            row.qfii_share_holding_percentage,
        )
    }
}

impl From<&crate::domain::registry::entity::Stock> for StockDbRow {
    fn from(stock: &crate::domain::registry::entity::Stock) -> Self {
        StockDbRow {
            stock_symbol: stock.symbol().0.clone(),
            name: stock.name().to_string(),
            suspend_listing: stock.suspend_listing(),
            net_asset_value_per_share: stock.net_asset_value_per_share(),
            weight: stock.weight(),
            return_on_equity: stock.return_on_equity(),
            create_time: stock.created_time(),
            stock_exchange_market_id: stock.market_id(),
            stock_industry_id: stock.industry_id(),
            issued_share: stock.issued_share(),
            qfii_shares_held: stock.qfii_shares_held(),
            qfii_share_holding_percentage: stock.qfii_share_holding_percentage(),
        }
    }
}

impl From<crate::domain::registry::entity::Stock> for StockDbRow {
    fn from(stock: crate::domain::registry::entity::Stock) -> Self {
        StockDbRow::from(&stock)
    }
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);

/// 取得未下市上市櫃每股淨值為零的股票
pub async fn fetch_net_asset_value_per_share_is_zero() -> Result<Vec<StockDbRow>> {
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
    AND s.stock_industry_id != $1
    AND s.net_asset_value_per_share = 0
"#;

    sqlx::query_as::<_, StockDbRow>(sql)
        .bind(Industry::ExchangeTradedFund.serial())
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_net_asset_value_per_share_is_zero from database")
}

/// 取得尚未有指定年度的季報的股票或者財報的每股淨值為零的股票
pub async fn fetch_stocks_without_financial_statement(
    year: i32,
    quarter: &str,
) -> Result<Vec<StockDbRow>> {
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
    AND s.stock_industry_id != $3
    AND NOT EXISTS (
        SELECT 1
        FROM financial_statement f
        WHERE f.security_code = s.stock_symbol
        AND f.year = $1
        AND f.quarter = $2
        AND earnings_per_share > 0
    )
"#;

    sqlx::query_as::<_, StockDbRow>(sql)
        .bind(year)
        .bind(quarter)
        .bind(Industry::ExchangeTradedFund.serial())
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
mod tests;
