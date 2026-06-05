use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Datelike, Local, TimeDelta};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, postgres::PgRow, Row};

use crate::{
    core::declare::Industry,
    core::logging,
    core::util::{self, map::Keyable},
    infra::crawler::{tpex, twse},
    infra::database,
};

pub(crate) mod extension;
pub mod stock_exchange_market;
pub(crate) mod stock_index;
pub mod stock_ownership_details;
pub(crate) mod stock_word;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 原表名 stocks
pub struct Stock {
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

impl Stock {
    /// 建立 `Stock` 預設值。
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

    /// 轉成推送給 Go stock service 的股票資訊請求。
    pub fn to_stock_info_request(&self) -> crate::interfaces::rpc::stock::StockInfoRequest {
        crate::interfaces::rpc::stock::StockInfoRequest {
            stock_symbol: self.stock_symbol.to_string(),
            name: self.name.to_string(),
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_industry_id: self.stock_industry_id,
            net_asset_value_per_share: self.net_asset_value_per_share.to_f64().unwrap_or(0.0),
            suspend_listing: self.suspend_listing,
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

    /// 新增或更新股票基本資訊 (Upsert)
    ///
    /// 在股票代號 (`stock_symbol`) 衝突時，會更新股票名稱、下市狀態等欄位。
    /// 為了防止部分爬蟲因未提供市場編號或產業分類（傳入 0）而將現有正確資料覆蓋為 0，
    /// SQL 語句中使用 `CASE WHEN` 進行防禦，只有當 EXCLUDED 值大於 0 時才進行更新。
    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO stocks (
    stock_symbol, "Name", "CreateTime",
    "SuspendListing", stock_exchange_market_id, stock_industry_id, weight)
VALUES ($1, $2, $3, $4, $5, $6, 0)
ON CONFLICT (stock_symbol) DO UPDATE SET
    "Name" = EXCLUDED."Name",
    "SuspendListing" = EXCLUDED."SuspendListing",
    -- 當傳入的市場編號大於 0 時才更新，否則保留原有的市場編號
    stock_exchange_market_id = CASE 
        WHEN EXCLUDED.stock_exchange_market_id > 0 THEN EXCLUDED.stock_exchange_market_id 
        ELSE stocks.stock_exchange_market_id 
    END,
    -- 當傳入的產業編號大於 0 時才更新，否則保留原有的產業編號
    stock_industry_id = CASE 
        WHEN EXCLUDED.stock_industry_id > 0 THEN EXCLUDED.stock_industry_id 
        ELSE stocks.stock_industry_id 
    END;
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
                    "Failed to Stock::fetch from database({}) because:{:?}",
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
            stock.create_index().await;
        }

        Ok(())
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
    AND s.stock_industry_id != $1
    AND s.net_asset_value_per_share = 0
"#;

    sqlx::query_as::<_, Stock>(sql)
        .bind(Industry::ExchangeTradedFund.serial())
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

    sqlx::query_as::<_, Stock>(sql)
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
mod tests {
    use rust_decimal_macros::dec;

    use crate::core::logging;
    use crate::infra::database::table::stock_exchange_market::StockExchangeMarket;

    use super::*;

    #[test]
    fn is_preference_shares_detects_letters_only() {
        assert!(!is_preference_shares("2330"));
        assert!(!is_preference_shares("0050"));
        assert!(is_preference_shares("2881A"));
        assert!(is_preference_shares("abc"));
        assert!(is_preference_shares("123b"));
    }

    #[test]
    fn stock_methods_key_tdr_and_stock_info_request() {
        let mut stock = Stock::new();
        stock.stock_symbol = "9105".to_string();
        stock.name = "泰金寶-DR".to_string();
        stock.stock_exchange_market_id = 2;
        stock.stock_industry_id = 20;
        stock.net_asset_value_per_share = dec!(12.34);
        stock.suspend_listing = true;

        let request = stock.to_stock_info_request();

        assert_eq!(stock.key(), "9105");
        assert_eq!(stock.key_with_prefix(), "Stock:9105");
        assert!(stock.is_tdr());
        assert_eq!(request.stock_symbol, "9105");
        assert_eq!(request.name, "泰金寶-DR");
        assert_eq!(request.stock_exchange_market_id, 2);
        assert_eq!(request.stock_industry_id, 20);
        assert_eq!(request.net_asset_value_per_share, 12.34);
        assert!(request.suspend_listing);
    }

    #[test]
    fn stock_clone_preserves_all_fields() {
        let mut stock = Stock::new();
        stock.stock_symbol = "2330".to_string();
        stock.name = "台積電".to_string();
        stock.suspend_listing = true;
        stock.net_asset_value_per_share = dec!(95.12);
        stock.weight = dec!(31.5);
        stock.return_on_equity = dec!(25.7);
        stock.stock_exchange_market_id = 2;
        stock.stock_industry_id = 24;
        stock.issued_share = 25_000;
        stock.qfii_shares_held = 12_000;
        stock.qfii_share_holding_percentage = dec!(48.5);

        let cloned = stock.clone();

        assert_eq!(cloned.stock_symbol, stock.stock_symbol);
        assert_eq!(cloned.name, stock.name);
        assert_eq!(cloned.suspend_listing, stock.suspend_listing);
        assert_eq!(
            cloned.net_asset_value_per_share,
            stock.net_asset_value_per_share
        );
        assert_eq!(cloned.weight, stock.weight);
        assert_eq!(cloned.return_on_equity, stock.return_on_equity);
        assert_eq!(cloned.create_time, stock.create_time);
        assert_eq!(
            cloned.stock_exchange_market_id,
            stock.stock_exchange_market_id
        );
        assert_eq!(cloned.stock_industry_id, stock.stock_industry_id);
        assert_eq!(cloned.issued_share, stock.issued_share);
        assert_eq!(cloned.qfii_shares_held, stock.qfii_shares_held);
        assert_eq!(
            cloned.qfii_share_holding_percentage,
            stock.qfii_share_holding_percentage
        );
    }

    #[test]
    fn stock_from_isin_maps_market_industry_and_identity() {
        let isin = twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            isin_code: "TW0002330008".to_string(),
            listing_date: "1994/09/05".to_string(),
            industry: "半導體業".to_string(),
            cfi_code: "ESVUFR".to_string(),
            exchange_market: StockExchangeMarket::new(2, 1),
            industry_id: 24,
        };

        let stock = Stock::from(isin);

        assert_eq!(stock.stock_symbol, "2330");
        assert_eq!(stock.name, "台積電");
        assert!(!stock.suspend_listing);
        assert_eq!(stock.stock_exchange_market_id, 2);
        assert_eq!(stock.stock_industry_id, 24);
        assert_eq!(stock.net_asset_value_per_share, Decimal::ZERO);
        assert_eq!(stock.weight, Decimal::ZERO);
        assert_eq!(stock.issued_share, 0);
    }

    #[test]
    fn stock_from_emerging_maps_nav_and_symbol() {
        let emerging =
            tpex::net_asset_value_per_share::Emerging::new("6987".to_string(), dec!(42.19));

        let stock = Stock::from(emerging);

        assert_eq!(stock.stock_symbol, "6987");
        assert_eq!(stock.name, "");
        assert_eq!(stock.net_asset_value_per_share, dec!(42.19));
        assert_eq!(stock.stock_exchange_market_id, 0);
        assert_eq!(stock.stock_industry_id, 0);
        assert!(!stock.suspend_listing);
    }

    // 此測試驗證防禦性 upsert 邏輯（當新傳入的市場或產業編號為 0 時，保留資料庫中原先正確的非零值）。
    #[tokio::test]
    async fn test_upsert_industry() {
        dotenv::dotenv().ok();

        // 若目前測試環境無法連接資料庫，則自動跳過，避免在無資料庫之開發環境下執行單元測試失敗
        if crate::infra::database::ping().await.is_err() {
            println!("跳過 test_upsert_industry：資料庫未啟動或無法連線");
            return;
        }

        let test_symbol = "__TEST_UPSERT_IND_9999__";
        let cleanup_sql = "DELETE FROM stocks WHERE stock_symbol = $1;";

        // 1. 確保乾淨的測試起點，先清除可能存在的舊測試資料
        sqlx::query(cleanup_sql)
            .bind(test_symbol)
            .execute(crate::infra::database::get_connection())
            .await
            .ok();

        // 2. 寫入初始的測試股票資料，並給予非零之市場與產業編號
        let mut seed = Stock::new();
        seed.stock_symbol = test_symbol.to_string();
        seed.name = "測試防禦更新股".to_string();
        seed.stock_exchange_market_id = 4; // 初始市場 ID
        seed.stock_industry_id = 15; // 初始產業 ID
        seed.upsert().await.expect("Failed to insert seed stock");

        // 3. 測試防禦邏輯：呼叫 upsert 時傳入 0 的市場與產業 ID
        let mut update_zero = Stock::new();
        update_zero.stock_symbol = test_symbol.to_string();
        update_zero.name = "測試防禦更新股_已更新".to_string();
        update_zero.stock_exchange_market_id = 0; // 傳入 0
        update_zero.stock_industry_id = 0; // 傳入 0
        update_zero
            .upsert()
            .await
            .expect("Failed to upsert zero fields");

        // 4. 驗證防禦更新：確認原先的非零值 (4 與 15) 被妥善保留，並未被 0 覆蓋
        let row1 = sqlx::query("SELECT \"Name\", stock_exchange_market_id, stock_industry_id FROM stocks WHERE stock_symbol = $1")
            .bind(test_symbol)
            .fetch_one(crate::infra::database::get_connection())
            .await
            .expect("Failed to fetch updated stock");
        assert_eq!(row1.get::<String, _>("Name"), "測試防禦更新股_已更新");
        assert_eq!(row1.get::<i32, _>("stock_exchange_market_id"), 4);
        assert_eq!(row1.get::<i32, _>("stock_industry_id"), 15);

        // 5. 測試正常覆寫：呼叫 upsert 時傳入新的非零之市場與產業 ID
        let mut update_new = Stock::new();
        update_new.stock_symbol = test_symbol.to_string();
        update_new.name = "測試防禦更新股_二次更新".to_string();
        update_new.stock_exchange_market_id = 2; // 新市場 ID
        update_new.stock_industry_id = 8; // 新產業 ID
        update_new
            .upsert()
            .await
            .expect("Failed to upsert new fields");

        // 6. 驗證覆寫更新：確認市場與產業 ID 已經被正確更新為新的值 (2 與 8)
        let row2 = sqlx::query("SELECT \"Name\", stock_exchange_market_id, stock_industry_id FROM stocks WHERE stock_symbol = $1")
            .bind(test_symbol)
            .fetch_one(crate::infra::database::get_connection())
            .await
            .expect("Failed to fetch second updated stock");
        assert_eq!(row2.get::<String, _>("Name"), "測試防禦更新股_二次更新");
        assert_eq!(row2.get::<i32, _>("stock_exchange_market_id"), 2);
        assert_eq!(row2.get::<i32, _>("stock_industry_id"), 8);

        // 7. 清理測試資料，還原資料庫狀態
        sqlx::query(cleanup_sql)
            .bind(test_symbol)
            .execute(crate::infra::database::get_connection())
            .await
            .expect("Failed to cleanup test stock");
    }

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

    #[tokio::test]
    #[ignore]
    async fn test_rebuild_search_indices() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 rebuild_search_indices".to_string());

        match Stock::rebuild_search_indices().await {
            Ok(_) => {
                logging::debug_file_async("完成 rebuild_search_indices".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to rebuild_search_indices because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 rebuild_search_indices".to_string());
    }
}
